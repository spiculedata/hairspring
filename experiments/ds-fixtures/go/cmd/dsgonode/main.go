// dsgonode is the Go victim of the kill-and-restart (crash) experiment: the same protocol, record log and report region
// as the C node dsnode, with the Go runtime's threads and netpoller.
//
//	HAIRSPRING_PROC=<node>.i<k> dsgonode --node=n1 --port=P --tmp=DIR [--recover-only]
//
// The variable only names the incarnation (no SDK is linked). Experiment fixture, not product code. Nothing here has
// been run under Hermit.
package main

import (
	"bufio"
	"flag"
	"fmt"
	"net"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"

	"github.com/spiculedata/hairspring/experiments/ds-fixtures/go/internal/ds"
)

func fatal(msg string) {
	fmt.Fprintln(os.Stderr, "error: "+msg)
	os.Exit(2)
}

type node struct {
	mu          sync.Mutex
	tag         string
	incarnation uint32
	log         *os.File
	region      *ds.Region
	table       ds.Table
}

// put appends one record with one write, counted by the region: started, write, done; then the caller acknowledges.
func (n *node) put(key int, value uint64, ack func()) {
	n.mu.Lock()
	defer n.mu.Unlock()
	rec := ds.EncodeRecord(n.incarnation, uint32(value/1000), uint32(value%1000), uint32(key), value)
	n.region.AddStarted()
	_, _ = n.log.Write(rec[:])
	n.region.AddDone()
	n.table.Set(key, value)
	ack()
	n.region.AddAcked()
}

func (n *node) get(key int) string {
	n.mu.Lock()
	defer n.mu.Unlock()
	if n.table.Present[key] {
		return "VAL " + strconv.FormatUint(n.table.Value[key], 10) + "\n"
	}
	return "NIL\n"
}

func main() {
	fs := flag.NewFlagSet("dsgonode", flag.ExitOnError)
	name := fs.String("node", "", "node name, for example n1")
	port := fs.Int("port", 0, "listen port")
	tmp := fs.String("tmp", "", "trial directory")
	recoverOnly := fs.Bool("recover-only", false, "recover, print the start line and exit")
	_ = fs.Parse(os.Args[1:])
	tag := os.Getenv("HAIRSPRING_PROC")
	if tag == "" || *name == "" || *tmp == "" || (*port == 0 && !*recoverOnly) {
		fatal("usage: HAIRSPRING_PROC=n1.i0 dsgonode --node=n1 --port=P --tmp=DIR")
	}
	inc, err := ds.IncarnationOf(tag)
	if err != nil {
		fatal(err.Error())
	}

	logFile, rec, err := ds.RecoverLog(filepath.Join(*tmp, "ds-data", *name+".log"))
	if err != nil {
		fatal("cannot recover the log")
	}
	ds.Emit(fmt.Sprintf("DSC0 v1 role=node tag=%s recovered=%d torn=%d\n", tag, rec.Recovered, rec.Torn))
	if *recoverOnly {
		return
	}
	region, err := ds.NewRegion(*tmp, tag)
	if err != nil {
		fatal("cannot create the region")
	}
	n := &node{tag: tag, incarnation: inc, log: logFile, region: region, table: rec.Table}

	ln, err := net.Listen("tcp", fmt.Sprintf("127.0.0.1:%d", *port))
	if err != nil {
		fatal("cannot listen")
	}
	quit := make(chan struct{})
	var once sync.Once
	handle := func(c net.Conn) {
		defer c.Close()
		r := bufio.NewReader(c)
		for {
			line, err := r.ReadString('\n')
			if err != nil {
				return
			}
			f := strings.Fields(line)
			switch {
			case len(f) == 3 && f[0] == "PUT":
				key := ds.KeyIndex(f[1])
				value, perr := strconv.ParseUint(f[2], 10, 64)
				if key < 0 || perr != nil {
					_, _ = c.Write([]byte("ERR\n"))
					continue
				}
				n.put(key, value, func() { _, _ = c.Write([]byte("OK\n")) })
			case len(f) == 2 && f[0] == "GET":
				key := ds.KeyIndex(f[1])
				if key < 0 {
					_, _ = c.Write([]byte("ERR\n"))
					continue
				}
				_, _ = c.Write([]byte(n.get(key)))
			case len(f) == 1 && f[0] == "QUIT":
				once.Do(func() { close(quit) })
			default:
				_, _ = c.Write([]byte("ERR\n"))
			}
		}
	}
	go func() {
		for {
			c, err := ln.Accept()
			if err != nil {
				return
			}
			go handle(c)
		}
	}()

	<-quit
	ds.Emit(fmt.Sprintf("DSC0 v1 role=node tag=%s exit=quit done=%d\n", tag, region.Done()))
	os.Exit(0)
}
