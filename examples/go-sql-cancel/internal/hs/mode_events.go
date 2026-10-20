//go:build !hs_violation_only

package hs

// violationOnlyBuild is false in the default build: Main writes the full event report.
const violationOnlyBuild = false
