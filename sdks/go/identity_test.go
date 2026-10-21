package hairspring

import (
	"strings"
	"testing"
)

const testPID = 4242

func TestResolveIdentity(t *testing.T) {
	cases := []struct {
		name     string
		env      map[string]string
		tag      string
		unstable bool
		refused  bool
	}{
		{"no variables", nil, "p1", false, false},
		{"tag from the harness, no owner", map[string]string{"HAIRSPRING_PROC": "p1.f2"}, "p1.f2", false, false},
		{"owner is another pid: a spawned child", map[string]string{"HAIRSPRING_PROC": "p1", "HAIRSPRING_PROC_PID": "1111"}, "p1.s4242", true, false},
		{"owner is this pid", map[string]string{"HAIRSPRING_PROC": "p1", "HAIRSPRING_PROC_PID": "4242"}, "p1", false, false},
		{"owner is this pid and announced: exec without fork", map[string]string{"HAIRSPRING_PROC": "p1", "HAIRSPRING_PROC_PID": "4242:a"}, "p1.x1", false, false},
		{"owner is this pid and unstable", map[string]string{"HAIRSPRING_PROC": "p1.s9", "HAIRSPRING_PROC_PID": "4242:u"}, "p1.s9", true, false},
		{"announced and unstable", map[string]string{"HAIRSPRING_PROC": "p1.s9", "HAIRSPRING_PROC_PID": "4242:au"}, "p1.s9.x1", true, false},
		{"owner field is garbage", map[string]string{"HAIRSPRING_PROC": "p1", "HAIRSPRING_PROC_PID": "garbage"}, "p1.s4242", true, false},
		{"owner field has an unknown flag", map[string]string{"HAIRSPRING_PROC": "p1", "HAIRSPRING_PROC_PID": "4242:z"}, "p1.s4242", true, false},
		{"empty tag with an owner", map[string]string{"HAIRSPRING_PROC": "", "HAIRSPRING_PROC_PID": "1111"}, "p1", false, false},
		{"tag with a space is refused", map[string]string{"HAIRSPRING_PROC": "has space"}, "p1", false, true},
		{"the reserved eval tag is refused", map[string]string{"HAIRSPRING_PROC": "_eval"}, "p1", false, true},
		{"tag over 64 bytes is refused", map[string]string{"HAIRSPRING_PROC": strings.Repeat("a", 65)}, "p1", false, true},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			got := resolveIdentity(env(c.env), testPID)

			if got.tag != c.tag || got.unstable != c.unstable || got.refused != c.refused {
				t.Fatalf("resolveIdentity = %+v, want tag=%q unstable=%v refused=%v", got, c.tag, c.unstable, c.refused)
			}
		})
	}
}

func TestADerivedTagThatWouldPassTheLimitKeepsAReadablePrefixAndStaysUnique(t *testing.T) {
	long := strings.Repeat("a", 64)
	environment := map[string]string{"HAIRSPRING_PROC": long, "HAIRSPRING_PROC_PID": "1"}

	one := resolveIdentity(env(environment), 100).tag
	two := resolveIdentity(env(environment), 101).tag

	if len(one) > 64 || !identPattern.MatchString(one) || !strings.HasPrefix(one, strings.Repeat("a", 40)+".h") {
		t.Fatalf("tag = %q", one)
	}
	if one == two {
		t.Fatalf("two children got the same tag %q", one)
	}
}

func TestOwnerValueCarriesThePidAndTheFlags(t *testing.T) {
	cases := []struct {
		announced, unstable bool
		want                string
	}{{false, false, "4242"}, {true, false, "4242:a"}, {false, true, "4242:u"}, {true, true, "4242:au"}}
	for _, c := range cases {
		if got := ownerValue(testPID, c.announced, c.unstable); got != c.want {
			t.Errorf("ownerValue(announced=%v, unstable=%v) = %q, want %q", c.announced, c.unstable, got, c.want)
		}
	}
}
