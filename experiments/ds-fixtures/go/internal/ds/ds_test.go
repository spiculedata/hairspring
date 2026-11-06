package ds

import (
	"bytes"
	"encoding/binary"
	"encoding/hex"
	"os"
	"path/filepath"
	"testing"
	"time"
)

// Golden values computed by the independent Python oracle (tests/oracle.py), not by this package.
const (
	goldenEmptyDigest = "cbf29ce484222325"
	goldenTwoKeys     = "975ec6eda8b0a8bf" // k0=5, k3=1003
	goldenRecord      = "3143534400000000010000000300000002000000eb0300000000000000000000"
	goldenHeartbeat   = "3143534402000000ffffffff07000000ff000000000000000000000000000000"
)

func TestDigestMatchesTheOracle(t *testing.T) {
	var empty Table
	if got := Hex16(empty.Digest()); got != goldenEmptyDigest {
		t.Fatalf("empty digest %s, want %s", got, goldenEmptyDigest)
	}
	var tab Table
	tab.Set(0, 5)
	tab.Set(3, 1003)
	if got := Hex16(tab.Digest()); got != goldenTwoKeys {
		t.Fatalf("digest %s, want %s", got, goldenTwoKeys)
	}
	if got := Hex16(Fnv(fnvOffset, []byte("a"))); got != "af63dc4c8601ec8c" {
		t.Fatalf("fnv of a: %s", got)
	}
}

func TestKeyIndex(t *testing.T) {
	cases := map[string]int{"k0": 0, "k7": 7, "k8": -1, "k": -1, "k00": -1, "x1": -1, "": -1}
	for in, want := range cases {
		if got := KeyIndex(in); got != want {
			t.Errorf("KeyIndex(%q) = %d, want %d", in, got, want)
		}
	}
}

func TestHandleProtocol(t *testing.T) {
	s := &Server{Node: "n1"}
	if a := s.Handle("PUT k3 7", true); a.Reply != "OK\n" || a.Replicate != "REPL k3 7\n" {
		t.Fatalf("PUT gave %+v", a)
	}
	if a := s.Handle("PUT k4 8", false); a.Reply != "OK\n" || a.Replicate != "" {
		t.Fatalf("PUT without a successor gave %+v", a)
	}
	if a := s.Handle("GET k3", true); a.Reply != "VAL 7\n" {
		t.Fatalf("GET gave %+v", a)
	}
	if a := s.Handle("GET k5", true); a.Reply != "NIL\n" {
		t.Fatalf("GET of a missing key gave %+v", a)
	}
	if a := s.Handle("REPL k5 9", true); a.Reply != "" || a.Replicate != "" {
		t.Fatalf("REPL gave %+v", a)
	}
	for _, bad := range []string{"BOGUS", "PUT k9 1", "PUT k1 x", "GET k9"} {
		if a := s.Handle(bad, true); a.Reply != "ERR\n" {
			t.Errorf("%q gave %+v, want ERR", bad, a)
		}
	}
	if a := s.Handle("HELLO n2", true); a != (Action{}) {
		t.Fatalf("HELLO gave %+v", a)
	}
	if a := s.Handle("QUIT", true); !a.Quit {
		t.Fatalf("QUIT gave %+v", a)
	}
	want := "DSN0 v1 role=srv node=n1 mode=go applied=2 repl_in=1 repl_out=1 refused=0 hb=0 suspect=0 " +
		"clock=0000000000000000 state="
	if got := s.ResultLine(); len(got) != len(want)+17 || got[:len(want)] != want {
		t.Fatalf("result line %q", got)
	}
}

func TestRecordEncodingMatchesTheOracle(t *testing.T) {
	rec := EncodeRecord(0, 1, 3, 2, 1003)
	if got := hex.EncodeToString(rec[:]); got != goldenRecord {
		t.Fatalf("record %s, want %s", got, goldenRecord)
	}
	hb := EncodeRecord(2, HeartbeatClient, 7, 0xFF, 0)
	if got := hex.EncodeToString(hb[:]); got != goldenHeartbeat {
		t.Fatalf("heartbeat %s, want %s", got, goldenHeartbeat)
	}
}

func TestRecoverLog(t *testing.T) {
	good := func(i int) []byte {
		r := EncodeRecord(0, 1, uint32(i%1000), uint32(i%8), uint64(1000+i))
		return r[:]
	}
	cases := []struct {
		name      string
		records   int
		tail      []byte
		recovered uint
		torn      uint
		sizeAfter int64
	}{
		{"empty", 0, nil, 0, 0, 0},
		{"one", 1, nil, 1, 0, 32},
		{"thousand", 1000, nil, 1000, 0, 32000},
		{"torn tail", 5, bytes.Repeat([]byte{1}, 7), 5, 1, 160},
		{"bad magic", 4, make([]byte, 32), 4, 1, 160},
	}
	for _, c := range cases {
		path := filepath.Join(t.TempDir(), "n1.log")
		var blob []byte
		for i := 0; i < c.records; i++ {
			blob = append(blob, good(i)...)
		}
		if err := os.WriteFile(path, append(blob, c.tail...), 0o644); err != nil {
			t.Fatal(err)
		}
		f, rec, err := RecoverLog(path)
		if err != nil {
			t.Fatalf("%s: %v", c.name, err)
		}
		f.Close()
		st, _ := os.Stat(path)
		if rec.Recovered != c.recovered || rec.Torn != c.torn || st.Size() != c.sizeAfter {
			t.Errorf("%s: recovered %d torn %d size %d, want %d %d %d", c.name, rec.Recovered, rec.Torn, st.Size(), c.recovered, c.torn, c.sizeAfter)
		}
	}
}

func TestRecoverLogRebuildsTheTableAndSkipsHeartbeats(t *testing.T) {
	path := filepath.Join(t.TempDir(), "n1.log")
	a, hb, b := EncodeRecord(0, 1, 0, 2, 1000), EncodeRecord(0, HeartbeatClient, 0, 2, 5), EncodeRecord(0, 1, 1, 2, 1001)
	blob := append(append(a[:], hb[:]...), b[:]...)
	if err := os.WriteFile(path, blob, 0o644); err != nil {
		t.Fatal(err)
	}
	f, rec, err := RecoverLog(path)
	if err != nil {
		t.Fatal(err)
	}
	f.Close()
	if !rec.Table.Present[2] || rec.Table.Value[2] != 1001 || rec.Recovered != 3 {
		t.Fatalf("table %+v recovered %d", rec.Table, rec.Recovered)
	}
}

func TestRegionHeaderAndCounters(t *testing.T) {
	dir := t.TempDir()
	if err := os.Mkdir(filepath.Join(dir, "hairspring-region"), 0o755); err != nil {
		t.Fatal(err)
	}
	r, err := NewRegion(dir, "n1.i4")
	if err != nil {
		t.Fatal(err)
	}
	r.AddStarted()
	r.AddStarted()
	r.AddDone()
	blob, err := os.ReadFile(filepath.Join(dir, "hairspring-region", "n1.i4"))
	if err != nil {
		t.Fatal(err)
	}
	if len(blob) != RegionBytes || string(blob[:8]) != "HSRGNv1\x00" {
		t.Fatalf("bad region header %q", blob[:16])
	}
	if binary.LittleEndian.Uint32(blob[8:]) != 1 || binary.LittleEndian.Uint32(blob[12:]) != 64 {
		t.Fatal("bad version or header size")
	}
	if string(bytes.TrimRight(blob[16:64], "\x00")) != "n1.i4" {
		t.Fatal("bad tag")
	}
	got := [3]uint64{binary.LittleEndian.Uint64(blob[64:]), binary.LittleEndian.Uint64(blob[72:]), binary.LittleEndian.Uint64(blob[80:])}
	if got != [3]uint64{2, 1, 0} || r.Started() != 2 || r.Done() != 1 || r.Acked() != 0 {
		t.Fatalf("counters %v", got)
	}
}

func TestNamesAndTagsAreValidatedBeforeTheyReachAPath(t *testing.T) {
	for _, ok := range []string{"n1", "N2", "abc123"} {
		if !ValidName(ok) {
			t.Errorf("%q should be accepted", ok)
		}
	}
	for _, bad := range []string{"", "../x", "n1/", "a.b", "n 1", "abcdefghijklmnopq"} {
		if ValidName(bad) {
			t.Errorf("%q should be rejected", bad)
		}
	}
	if !ValidTag("n1.i0", "n1") || !ValidTag("n1.i123", "n1") {
		t.Error("good tags rejected")
	}
	for _, bad := range []string{"n1.i", "n2.i0", "n1.i0/x", "n1.i-1", "../n1.i0", "n1.i1234567890"} {
		if ValidTag(bad, "n1") {
			t.Errorf("tag %q should be rejected", bad)
		}
	}
}

func TestWatchIdle(t *testing.T) {
	quit := make(chan struct{})
	activity := make(chan struct{}, 1)
	// No signal: the guard fires.
	if !WatchIdle(20*time.Millisecond, activity, quit) {
		t.Fatal("the guard should fire with no request")
	}
	// Requests keep it quiet; quit ends it without firing.
	done := make(chan bool)
	go func() { done <- WatchIdle(60*time.Millisecond, activity, quit) }()
	for i := 0; i < 5; i++ {
		time.Sleep(20 * time.Millisecond)
		activity <- struct{}{}
	}
	close(quit)
	if <-done {
		t.Fatal("quit must end the watch without the guard firing")
	}
	if IdleLimit != 120*time.Second {
		t.Fatalf("idle limit %v", IdleLimit)
	}
}

func TestIncarnationOf(t *testing.T) {
	if n, err := IncarnationOf("n1.i12"); err != nil || n != 12 {
		t.Fatalf("got %d %v", n, err)
	}
	for _, bad := range []string{"", "n1", "n1.i", ".i3", "n1.ix"} {
		if _, err := IncarnationOf(bad); err == nil {
			t.Errorf("%q should be rejected", bad)
		}
	}
}
