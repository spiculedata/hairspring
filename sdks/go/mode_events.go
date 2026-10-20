//go:build !hs_violation_only

package hairspring

// violationOnlyBuild is false in the default build: Main writes the full event report.
const violationOnlyBuild = false
