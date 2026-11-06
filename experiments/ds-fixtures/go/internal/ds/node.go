package ds

import (
	"encoding/binary"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sync/atomic"
	"syscall"
	"unsafe"
)

// Record and region layout, shared with the C node (dsnode) and the Python oracle.
const (
	RecordBytes       = 32
	RecordMagic       = 0x44534331
	HeartbeatClient   = 0xFFFFFFFF
	RegionBytes       = 4096
	RegionHeaderBytes = 64
)

var regionMagic = [8]byte{'H', 'S', 'R', 'G', 'N', 'v', '1', 0}

// EncodeRecord builds one 32-byte little-endian log record (the last four bytes are reserved zeros).
func EncodeRecord(incarnation, client, op, key uint32, value uint64) [RecordBytes]byte {
	var b [RecordBytes]byte
	binary.LittleEndian.PutUint32(b[0:], RecordMagic)
	binary.LittleEndian.PutUint32(b[4:], incarnation)
	binary.LittleEndian.PutUint32(b[8:], client)
	binary.LittleEndian.PutUint32(b[12:], op)
	binary.LittleEndian.PutUint32(b[16:], key)
	binary.LittleEndian.PutUint64(b[20:], value)
	return b
}

// Recovery is what reading the log back gave.
type Recovery struct {
	Recovered uint
	Torn      uint
	Table     Table
}

// RecoverLog opens (creating if needed) the append-only log, counts valid records and torn entries, truncates a partial
// tail and rebuilds the key table. The returned file appends.
func RecoverLog(path string) (*os.File, Recovery, error) {
	var rec Recovery
	f, err := os.OpenFile(path, os.O_RDWR|os.O_CREATE|os.O_APPEND, 0o644)
	if err != nil {
		return nil, rec, err
	}
	data, err := os.ReadFile(path)
	if err != nil {
		f.Close()
		return nil, rec, err
	}
	full := len(data) / RecordBytes
	for n := 0; n < full; n++ {
		b := data[n*RecordBytes : (n+1)*RecordBytes]
		if binary.LittleEndian.Uint32(b[0:]) != RecordMagic {
			rec.Torn++
			continue
		}
		rec.Recovered++
		client, key := binary.LittleEndian.Uint32(b[8:]), binary.LittleEndian.Uint32(b[16:])
		if client != HeartbeatClient && key < Keys {
			rec.Table.Set(int(key), binary.LittleEndian.Uint64(b[20:]))
		}
	}
	if len(data)%RecordBytes != 0 {
		rec.Torn++
		if err := f.Truncate(int64(full * RecordBytes)); err != nil {
			f.Close()
			return nil, rec, err
		}
	}
	return f, rec, nil
}

// Region is the 4,096-byte report region of one incarnation, mapped shared. After NewRegion returns no system call
// touches it: counters move with atomic adds.
type Region struct {
	mem []byte
}

// NewRegion creates <dir>/hairspring-region/<tag>, writes the header and maps it.
func NewRegion(dir, tag string) (*Region, error) {
	if len(tag) >= 48 {
		return nil, errors.New("tag too long")
	}
	f, err := os.OpenFile(filepath.Join(dir, "hairspring-region", tag), os.O_RDWR|os.O_CREATE|os.O_TRUNC, 0o644)
	if err != nil {
		return nil, err
	}
	defer f.Close()
	if err := f.Truncate(RegionBytes); err != nil {
		return nil, err
	}
	mem, err := syscall.Mmap(int(f.Fd()), 0, RegionBytes, syscall.PROT_READ|syscall.PROT_WRITE, syscall.MAP_SHARED)
	if err != nil {
		return nil, err
	}
	copy(mem[0:8], regionMagic[:])
	binary.LittleEndian.PutUint32(mem[8:], 1)
	binary.LittleEndian.PutUint32(mem[12:], RegionHeaderBytes)
	copy(mem[16:], tag)
	return &Region{mem: mem}, nil
}

func (r *Region) counter(i int) *uint64 {
	return (*uint64)(unsafe.Pointer(&r.mem[RegionHeaderBytes+8*i]))
}

// Started, Done and Acked are the three counters at offset 64.
func (r *Region) Started() uint64 { return atomic.LoadUint64(r.counter(0)) }
func (r *Region) Done() uint64    { return atomic.LoadUint64(r.counter(1)) }
func (r *Region) Acked() uint64   { return atomic.LoadUint64(r.counter(2)) }

// AddStarted, AddDone and AddAcked increment a counter.
func (r *Region) AddStarted() { atomic.AddUint64(r.counter(0), 1) }
func (r *Region) AddDone()    { atomic.AddUint64(r.counter(1), 1) }
func (r *Region) AddAcked()   { atomic.AddUint64(r.counter(2), 1) }

// ValidName reports whether s is 1 to 16 letters and digits, safe to use in a path.
func ValidName(s string) bool {
	if len(s) == 0 || len(s) > 16 {
		return false
	}
	for _, c := range s {
		if !(c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z' || c >= '0' && c <= '9') {
			return false
		}
	}
	return true
}

// ValidTag reports whether tag is exactly "<node>.i<digits>" for the given node name.
func ValidTag(tag, node string) bool {
	prefix := node + ".i"
	if !ValidName(node) || len(tag) <= len(prefix) || len(tag) > len(prefix)+9 || tag[:len(prefix)] != prefix {
		return false
	}
	for _, c := range tag[len(prefix):] {
		if c < '0' || c > '9' {
			return false
		}
	}
	return true
}

// IncarnationOf parses the incarnation number from a tag like "n1.i3".
func IncarnationOf(tag string) (uint32, error) {
	var node string
	var inc uint32
	for i := 0; i+1 < len(tag); i++ {
		if tag[i] == '.' && tag[i+1] == 'i' {
			node = tag[:i]
			if _, err := fmt.Sscanf(tag[i+2:], "%d", &inc); err != nil || node == "" {
				break
			}
			return inc, nil
		}
	}
	return 0, errors.New("HAIRSPRING_PROC must look like n1.i0")
}
