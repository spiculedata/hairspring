//go:build hs_violation_only

package hairspring

// violationOnlyBuild is true under `-tags hs_violation_only`: Main writes only HAIRSPRING_VIOLATION: lines. A build
// like this can be compared with the default build to check that the report does not change the program's behaviour.
const violationOnlyBuild = true
