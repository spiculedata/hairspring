package retaindrv

import (
	"context"
	"database/sql/driver"
	"errors"
	"io"
	"sync/atomic"
)

// Hooks let the workload observe the driver without the driver knowing about the workload. Both are optional and
// are called from whichever goroutine database/sql uses, so they must be safe for concurrent use and must not
// block or call back into database/sql.
type Hooks struct {
	// OnQuery runs in QueryContext. prevOwner is the id of the query that owned the connection until now, or 0
	// if none did: anything but 0 means two queries overlapped on the one connection.
	OnQuery func(qid, prevOwner int64)
	// OnNext runs in Rows.Next, before the connection buffer is rewritten with the row of query qid.
	OnNext func(qid int64)
}

// Connector is a driver.Connector for the fake driver. Use it with sql.OpenDB; nothing is registered globally.
type Connector struct {
	hooks     Hooks
	nextCalls atomic.Uint64
}

func NewConnector(h Hooks) *Connector { return &Connector{hooks: h} }

// Connect returns a new connection with its own row buffer.
func (c *Connector) Connect(context.Context) (driver.Conn, error) { return &conn{c: c}, nil }

// Driver returns a driver that refuses to open by name: the workload always goes through the connector.
func (c *Connector) Driver() driver.Driver { return nameDriver{} }

// NextCalls is the number of Rows.Next calls so far (including the final io.EOF ones).
func (c *Connector) NextCalls() uint64 { return c.nextCalls.Load() }

type nameDriver struct{}

func (nameDriver) Open(string) (driver.Conn, error) {
	return nil, errors.New("retaindrv: open it through Connector")
}

var errUnsupported = errors.New("retaindrv: unsupported")

type conn struct {
	c     *Connector
	buf   [RowLen]byte // the one buffer every row of every query on this connection is written into
	owner atomic.Int64 // id of the query holding the connection, 0 if none
}

func (*conn) Prepare(string) (driver.Stmt, error) { return nil, errUnsupported }
func (*conn) Close() error                        { return nil }
func (*conn) Begin() (driver.Tx, error)           { return nil, errUnsupported }

// QueryContext takes the query id from args[0] (an int64 in 1..MaxQueryID). It does not write the buffer: the
// row is written by Next, which is what lets a later query overwrite an earlier query's data.
func (cn *conn) QueryContext(_ context.Context, _ string, args []driver.NamedValue) (driver.Rows, error) {
	if len(args) != 1 {
		return nil, errors.New("retaindrv: want exactly one argument, the query id")
	}
	qid, ok := args[0].Value.(int64)
	if !ok || qid < 1 || qid > MaxQueryID {
		return nil, errors.New("retaindrv: the query id must be an int64 in 1..MaxQueryID")
	}
	prev := cn.owner.Swap(qid)
	if h := cn.c.hooks.OnQuery; h != nil {
		h(qid, prev)
	}
	return &rows{cn: cn, qid: qid}, nil
}

type rows struct {
	cn   *conn
	qid  int64
	done bool
}

func (*rows) Columns() []string { return []string{"row"} }

// Close records that the query no longer owns the connection and leaves the buffer alone: the property under
// test needs a later Next to overwrite the data, not a Close that poisons it.
func (r *rows) Close() error {
	r.cn.owner.CompareAndSwap(r.qid, 0)
	return nil
}

// Next writes the row of this query into the connection buffer and returns a slice of that buffer.
func (r *rows) Next(dest []driver.Value) error {
	r.cn.c.nextCalls.Add(1)
	if r.done {
		return io.EOF
	}
	r.done = true
	if h := r.cn.c.hooks.OnNext; h != nil {
		h(r.qid)
	}
	var row [RowLen]byte
	copy(r.cn.buf[:], AppendRow(row[:0], r.qid, 0))
	dest[0] = r.cn.buf[:RowLen:RowLen]
	return nil
}
