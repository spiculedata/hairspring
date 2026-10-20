package retaindrv

import (
	"context"
	"database/sql"
	"database/sql/driver"
	"io"
	"sync"
	"testing"
)

func mustConn(t *testing.T, c *Connector) driver.QueryerContext {
	t.Helper()
	conn, err := c.Connect(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	return conn.(driver.QueryerContext)
}

func query(t *testing.T, q driver.QueryerContext, qid int64) driver.Rows {
	t.Helper()
	rows, err := q.QueryContext(context.Background(), "q", []driver.NamedValue{{Ordinal: 1, Value: qid}})
	if err != nil {
		t.Fatal(err)
	}
	return rows
}

func next(t *testing.T, rows driver.Rows) []byte {
	t.Helper()
	dest := make([]driver.Value, 1)
	if err := rows.Next(dest); err != nil {
		t.Fatal(err)
	}
	return dest[0].([]byte)
}

func TestEachQueryYieldsOneRowOfItsOwnThenEOF(t *testing.T) {
	q := mustConn(t, NewConnector(Hooks{}))
	rows := query(t, q, 7)

	got := string(next(t, rows))

	if want := "q00000007-r0000-q00000007"; got != want {
		t.Fatalf("row = %q, want %q", got, want)
	}
	if err := rows.Next(make([]driver.Value, 1)); err != io.EOF {
		t.Fatalf("second Next = %v, want io.EOF", err)
	}
	if cols := rows.Columns(); len(cols) != 1 {
		t.Fatalf("columns = %v, want one", cols)
	}
}

func TestNextReturnsASliceOfTheConnectionBufferNotACopy(t *testing.T) {
	q := mustConn(t, NewConnector(Hooks{}))
	a := query(t, q, 1)
	rowA := next(t, a)
	if err := a.Close(); err != nil {
		t.Fatal(err)
	}

	b := query(t, q, 2)
	rowB := next(t, b)

	if string(rowA) != "q00000002-r0000-q00000002" {
		t.Fatalf("A's slice was not rewritten by B's Next: %q", rowA)
	}
	if &rowA[0] != &rowB[0] {
		t.Fatal("rows of two queries do not share one buffer")
	}
}

func TestCloseDoesNotTouchTheBuffer(t *testing.T) {
	q := mustConn(t, NewConnector(Hooks{}))
	a := query(t, q, 3)
	row := next(t, a)
	before := string(row)

	if err := a.Close(); err != nil {
		t.Fatal(err)
	}

	if string(row) != before {
		t.Fatalf("Close changed the buffer: %q -> %q", before, row)
	}
}

func TestOnQueryReportsThePreviousOwner(t *testing.T) {
	var prevs []int64
	q := mustConn(t, NewConnector(Hooks{OnQuery: func(qid, prev int64) { prevs = append(prevs, prev) }}))

	a := query(t, q, 1)
	b := query(t, q, 2) // A is still open: not exclusive
	_ = b.Close()
	_ = a.Close() // A no longer owns the connection; closing must not clear B's release
	c := query(t, q, 3)
	_ = c.Close()

	if len(prevs) != 3 || prevs[0] != 0 || prevs[1] != 1 || prevs[2] != 0 {
		t.Fatalf("previous owners = %v, want [0 1 0]", prevs)
	}
}

func TestOnNextRunsBeforeTheBufferIsRewritten(t *testing.T) {
	var seen string
	var row []byte
	q := mustConn(t, NewConnector(Hooks{OnNext: func(qid int64) {
		if row != nil {
			seen = string(row)
		}
	}}))
	a := query(t, q, 1)
	row = next(t, a)
	_ = a.Close()

	b := query(t, q, 2)
	_ = next(t, b)

	if seen != "q00000001-r0000-q00000001" {
		t.Fatalf("hook saw %q, want A's row still intact", seen)
	}
}

func TestNextCallsAreCounted(t *testing.T) {
	c := NewConnector(Hooks{})
	q := mustConn(t, c)
	for i := int64(1); i <= 3; i++ {
		r := query(t, q, i)
		next(t, r)
		_ = r.Close()
	}
	if got := c.NextCalls(); got != 3 {
		t.Fatalf("NextCalls = %d, want 3", got)
	}
}

func TestQueryContextRejectsBadArguments(t *testing.T) {
	q := mustConn(t, NewConnector(Hooks{}))
	for name, args := range map[string][]driver.NamedValue{
		"none":        nil,
		"two":         {{Ordinal: 1, Value: int64(1)}, {Ordinal: 2, Value: int64(2)}},
		"wrong type":  {{Ordinal: 1, Value: "x"}},
		"zero id":     {{Ordinal: 1, Value: int64(0)}},
		"too big id":  {{Ordinal: 1, Value: int64(MaxQueryID + 1)}},
		"negative id": {{Ordinal: 1, Value: int64(-4)}},
	} {
		if _, err := q.QueryContext(context.Background(), "q", args); err == nil {
			t.Errorf("%s: expected an error", name)
		}
	}
}

func TestUnsupportedConnMethodsFail(t *testing.T) {
	c := NewConnector(Hooks{})
	conn, _ := c.Connect(context.Background())
	if _, err := conn.Prepare("q"); err == nil {
		t.Error("Prepare should fail")
	}
	if _, err := conn.Begin(); err == nil {
		t.Error("Begin should fail")
	}
	if err := conn.Close(); err != nil {
		t.Errorf("Close = %v", err)
	}
	if _, err := c.Driver().Open("x"); err == nil {
		t.Error("Driver().Open should fail: use the connector")
	}
}

func TestWorksThroughDatabaseSQLWithAPoolOfOne(t *testing.T) {
	c := NewConnector(Hooks{})
	db := sql.OpenDB(c)
	defer db.Close()
	db.SetMaxOpenConns(1)

	var wg sync.WaitGroup
	for qid := int64(1); qid <= 4; qid++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			rows, err := db.QueryContext(context.Background(), "q", qid)
			if err != nil {
				t.Error(err)
				return
			}
			defer rows.Close()
			var got string
			if !rows.Next() || rows.Scan(&got) != nil {
				t.Errorf("qid %d: no row (%v)", qid, rows.Err())
				return
			}
			if want := string(AppendRow(nil, qid, 0)); got != want {
				t.Errorf("qid %d: got %q want %q", qid, got, want)
			}
		}()
	}
	wg.Wait()
}
