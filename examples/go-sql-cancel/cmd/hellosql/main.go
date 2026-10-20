// Command hellosql links database/sql, so its binary differs between toolchains whose database/sql differs.
// The plain hello program does not, which is why the build records hash both.
package main

import (
	"database/sql"
	"fmt"
)

func main() {
	fmt.Println("registered drivers:", len(sql.Drivers()))
}
