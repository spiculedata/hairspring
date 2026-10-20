package sqlcancel

import "fmt"

// Summary is what the run counted. Every field is deterministic given the schedule, none is a time or an address,
// so the stdout line is part of the behaviour hash and tells seeds apart.
type Summary struct {
	Mode        Mode
	Queries     uint64 // queries run
	ScanOK      uint64 // Scan calls that returned nil
	ScanErrors  uint64 // Scan calls that returned an error (allowed after a cancel)
	ForeignHead uint64 // successful scans whose head id is not the query's own
	Torn        uint64 // successful scans whose head and tail ids differ
	Unparsable  uint64 // successful scans that are not a row
	Overlapped  uint64 // scans during which the cancel flag flipped
	ForeignNext uint64 // driver Next calls made while another query's Scan was in flight
}

// Line is the one fixed-width stdout line: its length does not depend on the outcome.
func (s Summary) Line() string {
	return fmt.Sprintf("sqlcancel mode=%s queries=%8d scan_ok=%8d scan_err=%8d foreign_head=%8d torn=%8d unparsable=%8d overlapped=%8d foreign_next=%8d\n",
		s.Mode, s.Queries, s.ScanOK, s.ScanErrors, s.ForeignHead, s.Torn, s.Unparsable, s.Overlapped, s.ForeignNext)
}

// Result is a finished run: the summary plus the number of harness invariants that failed.
type Result struct {
	Summary
	HarnessFailures uint64
}

// ExitStatus is 3 when a harness invariant failed and 0 otherwise. The property result never changes it: a
// violation is reported only through the protocol.
func (r Result) ExitStatus() int {
	if r.HarnessFailures > 0 {
		return ExitHarnessFailure
	}
	return 0
}
