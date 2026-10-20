// Command sqlcancel races database/sql query cancellation against Rows.Scan over a driver that reuses its row
// buffer (the behaviour fixed upstream in go1.24.6, commit 83b4a5db2409). Usage: sqlcancel [-mode=std|yield] [-q N].
// Exit status: 0 on completion whatever the property result (it is reported through the guest protocol on
// standard error), 3 if a harness invariant failed, 64 on a usage error.
package main

import (
	"fmt"
	"os"

	"github.com/spiculedata/hairspring/examples/go-sql-cancel/internal/hs"
	"github.com/spiculedata/hairspring/examples/go-sql-cancel/internal/sqlcancel"
)

func main() {
	cfg, err := sqlcancel.ParseArgs(os.Args[1:])
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(sqlcancel.ExitUsage)
	}
	sites := sqlcancel.DeclareSites(hs.Declare)
	os.Exit(hs.Main(func() int {
		res := sqlcancel.Run(cfg, sites)
		hs.AddWork(res.Queries)
		fmt.Print(res.Line())
		return res.ExitStatus()
	}))
}
