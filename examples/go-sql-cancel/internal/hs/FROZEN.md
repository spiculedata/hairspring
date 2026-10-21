# Frozen: do not fix here

This package is the copy of the Go shim that the experiment's recorded binaries were built from (`BUILD-RECORD` at the
top of the example). Changing any `.go` file in this directory, even a comment, can change those binaries and so
invalidate the recorded hashes. Fixes and features go to the maintained SDK in `sdks/go` instead.

Known differences from the SDK, left as they are: no events file, no child-process tags, no cap on `end.sites`, a
snapshot that can pair a witness with a count of zero under concurrent evaluation, and writes to fd 2 directly.
