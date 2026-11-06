// dsgo is the Go server of the loopback experiment ring: the same line protocol as the C server dskv, using the Go netpoller (one
// goroutine per connection) and ring replication over one persistent link to the successor.
//
//	dsgo --node=n1 --port=P --peer-port=Q [--tmp=DIR] [--mode=...]
//
// --tmp and --mode are accepted so that the launcher can pass the same arguments to every server; they are ignored.
// Experiment fixture, not product code. Nothing here has been run under Hermit.
package main

import (
	"bufio"
	"errors"
	"flag"
	"fmt"
	"net"
	"os"
	"strings"
	"sync"
	"syscall"
	"time"

	"github.com/spiculedata/hairspring/experiments/ds-fixtures/go/internal/ds"
)

const (
	connectTries = 200
	retrySleep   = 5 * time.Millisecond
)

func fatal(msg string) {
	fmt.Fprintln(os.Stderr, "error: "+msg)
	os.Exit(2)
}

func connectSuccessor(srv *ds.Server, port int) net.Conn {
	addr := fmt.Sprintf("127.0.0.1:%d", port)
	for attempt := 0; attempt < connectTries; attempt++ {
		c, err := net.Dial("tcp", addr)
		if err == nil {
			return c
		}
		if !errors.Is(err, syscall.ECONNREFUSED) {
			fatal("cannot reach the successor")
		}
		srv.CountRefused()
		time.Sleep(retrySleep)
	}
	fatal("cannot reach the successor")
	return nil
}

func main() {
	fs := flag.NewFlagSet("dsgo", flag.ExitOnError)
	node := fs.String("node", "", "node name, for example n1")
	port := fs.Int("port", 0, "listen port")
	peerPort := fs.Int("peer-port", 0, "ring successor's port")
	fs.String("tmp", "", "ignored")
	fs.String("mode", "", "ignored")
	_ = fs.Parse(os.Args[1:])
	if *node == "" || *port <= 0 || *port > 65535 || *peerPort <= 0 || *peerPort > 65535 {
		fatal("usage: dsgo --node=n1 --port=P --peer-port=Q")
	}

	srv := &ds.Server{Node: *node}
	ln, err := net.Listen("tcp", fmt.Sprintf("127.0.0.1:%d", *port))
	if err != nil {
		fatal("cannot listen")
	}
	succ := connectSuccessor(srv, *peerPort)
	if _, err := succ.Write([]byte("HELLO " + *node + "\n")); err != nil {
		fatal("cannot reach the successor")
	}
	// Anything the successor sends back (heartbeats from other implementations) is read and dropped.
	go func() { _, _ = bufio.NewReader(succ).WriteTo(discard{}) }()

	var sendMu sync.Mutex
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
			act := srv.Handle(strings.TrimRight(line, "\r\n"), true)
			if act.Replicate != "" {
				sendMu.Lock()
				_, _ = succ.Write([]byte(act.Replicate))
				sendMu.Unlock()
			}
			if act.Reply != "" {
				_, _ = c.Write([]byte(act.Reply))
			}
			if act.Quit {
				once.Do(func() { close(quit) })
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
	ds.Emit(srv.ResultLine())
	os.Exit(0)
}

type discard struct{}

func (discard) Write(p []byte) (int, error) { return len(p), nil }
