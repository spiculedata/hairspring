// Package ds holds the logic shared by the Go DS experiment fixtures (dsgo, dsgonode): the key table and its digest,
// the key-value line protocol, and one-write output lines. Experiment fixtures, not product code; nothing here has
// been run under Hermit.
//
// Fixture rules: finite timeouts only, numeric loopback addresses, no randomness (the table is an array, never a map),
// every output line written with one write, no pid, port, path or clock value in a line.
package ds

import (
	"os"
	"strconv"
	"strings"
	"sync"
)

// Keys is the number of keys k0..k7.
const Keys = 8

const (
	fnvOffset uint64 = 0xcbf29ce484222325
	fnvPrime  uint64 = 0x100000001b3
)

// Fnv folds data into an FNV-1a 64 hash.
func Fnv(h uint64, data []byte) uint64 {
	for _, b := range data {
		h = (h ^ uint64(b)) * fnvPrime
	}
	return h
}

// Table is the key table: a fixed array so that nothing depends on map iteration order.
type Table struct {
	Present [Keys]bool
	Value   [Keys]uint64
}

// Set stores a value.
func (t *Table) Set(key int, value uint64) {
	t.Present[key] = true
	t.Value[key] = value
}

// Digest is the FNV-1a 64 of the text "k<d>=<value>;" for each present key in index order.
func (t *Table) Digest() uint64 {
	h := fnvOffset
	for k := 0; k < Keys; k++ {
		if t.Present[k] {
			h = Fnv(h, []byte("k"+strconv.Itoa(k)+"="+strconv.FormatUint(t.Value[k], 10)+";"))
		}
	}
	return h
}

// KeyIndex parses "k<d>" with d in 0..7, or returns -1.
func KeyIndex(s string) int {
	if len(s) != 2 || s[0] != 'k' || s[1] < '0' || s[1] >= '0'+Keys {
		return -1
	}
	return int(s[1] - '0')
}

// Emit writes one line to stdout with a single write.
func Emit(line string) {
	_, _ = os.Stdout.Write([]byte(line))
}

// Server is the loopback experiment key-value state of one node.
type Server struct {
	Node    string
	mu      sync.Mutex
	table   Table
	Applied uint64
	ReplIn  uint64
	ReplOut uint64
	refused uint
}

// Action is what the connection handler must do after one request line.
type Action struct {
	Reply     string // sent back on the connection ("" for none)
	Replicate string // sent to the ring successor ("" for none)
	Quit      bool
}

// Handle applies one request line. When successor is true a PUT is replicated.
func (s *Server) Handle(line string, successor bool) Action {
	f := strings.Fields(line)
	if len(f) == 0 {
		return Action{}
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	switch {
	case (f[0] == "PUT" || f[0] == "REPL") && len(f) == 3:
		key, value, err := parseKV(f[1], f[2])
		if err != nil {
			if f[0] == "PUT" {
				return Action{Reply: "ERR\n"}
			}
			return Action{}
		}
		s.table.Set(key, value)
		if f[0] == "REPL" {
			s.ReplIn++
			return Action{}
		}
		s.Applied++
		act := Action{Reply: "OK\n"}
		if successor {
			s.ReplOut++
			act.Replicate = "REPL " + f[1] + " " + f[2] + "\n"
		}
		return act
	case f[0] == "GET" && len(f) == 2:
		key := KeyIndex(f[1])
		switch {
		case key < 0:
			return Action{Reply: "ERR\n"}
		case s.table.Present[key]:
			return Action{Reply: "VAL " + strconv.FormatUint(s.table.Value[key], 10) + "\n"}
		default:
			return Action{Reply: "NIL\n"}
		}
	case f[0] == "HELLO" || f[0] == "HB":
		return Action{}
	case f[0] == "QUIT":
		return Action{Quit: true}
	}
	return Action{Reply: "ERR\n"}
}

// CountRefused records one refused connect to the successor.
func (s *Server) CountRefused() {
	s.mu.Lock()
	s.refused++
	s.mu.Unlock()
}

// ResultLine is the server's DSN0 result line, with mode "go".
func (s *Server) ResultLine() string {
	s.mu.Lock()
	defer s.mu.Unlock()
	return "DSN0 v1 role=srv node=" + s.Node + " mode=go applied=" + strconv.FormatUint(s.Applied, 10) +
		" repl_in=" + strconv.FormatUint(s.ReplIn, 10) + " repl_out=" + strconv.FormatUint(s.ReplOut, 10) +
		" refused=" + strconv.FormatUint(uint64(s.refused), 10) + " hb=0 suspect=0 clock=0000000000000000 state=" +
		hex16(s.table.Digest()) + "\n"
}

func parseKV(k, v string) (int, uint64, error) {
	key := KeyIndex(k)
	if key < 0 {
		return 0, 0, strconv.ErrSyntax
	}
	value, err := strconv.ParseUint(v, 10, 64)
	return key, value, err
}

func hex16(v uint64) string {
	s := strconv.FormatUint(v, 16)
	return strings.Repeat("0", 16-len(s)) + s
}

// Hex16 formats v as 16 lowercase hex digits.
func Hex16(v uint64) string { return hex16(v) }
