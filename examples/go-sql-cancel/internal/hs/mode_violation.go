//go:build hs_violation_only

package hs

// violationOnlyBuild is true under `-tags hs_violation_only`: Main writes only HAIRSPRING_VIOLATION: lines. The
// neutrality gate compares this build's results with the default build's.
const violationOnlyBuild = true
