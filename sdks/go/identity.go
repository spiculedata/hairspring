package hairspring

import (
	"fmt"
	"hash/fnv"
	"strconv"
	"strings"
)

// A process's tag lives in the environment as HAIRSPRING_PROC, with the owning pid and two flags in
// HAIRSPRING_PROC_PID ("<pid>", "<pid>:a", "<pid>:u", "<pid>:au"; a = the tag has been announced with a hello,
// u = the tag is not stable across runs). Both are set at startup, so every child process inherits them. This is the
// C SDK's scheme (sdks/c/src/hs_identity.c), so a Go and a C program in one process tree agree:
//
//   - root process: HAIRSPRING_PROC if the harness set it, else "p1";
//   - spawned child (os/exec, anything that did not fork from this image): the inherited pid is not its own, so it
//     takes "<inherited>.s<pid>" and the tag is flagged unstable (a pid differs between runs outside Hermit);
//   - exec without fork (syscall.Exec): the inherited pid is its own and the tag was announced, so the new image
//     takes "<tag>.x1".
//
// Go cannot fork, so there is no "<parent>.f<n>" case.
const (
	rootTag         = "p1"
	evalTag         = "_eval" // reserved for eval lines
	maxTagBytes     = 64
	hashPrefixBytes = 40 // readable part kept when a derived tag would pass maxTagBytes
	hashMix         = 0x9e3779b97f4a7c15
)

type identity struct {
	tag      string
	unstable bool // the tag was derived from a pid
	refused  bool // HAIRSPRING_PROC was set to something that is not a valid tag
}

func tagValid(t string) bool {
	return t != "" && len(t) <= maxTagBytes && identPattern.MatchString(t) && t != evalTag
}

func resolveIdentity(getenv func(string) string, pid int) identity {
	inherited := getenv("HAIRSPRING_PROC")
	if inherited == "" {
		return identity{tag: rootTag}
	}
	if !tagValid(inherited) {
		return identity{tag: rootTag, refused: true}
	}
	owner, present := parseOwner(getenv("HAIRSPRING_PROC_PID"))
	switch {
	case !present:
		return identity{tag: inherited}
	case !owner.valid || owner.pid != uint64(pid):
		return identity{tag: deriveTag(inherited, 's', uint64(pid)), unstable: true}
	case owner.announced:
		return identity{tag: deriveTag(inherited, 'x', 1), unstable: owner.unstable}
	default:
		return identity{tag: inherited, unstable: owner.unstable}
	}
}

type owner struct {
	valid               bool
	pid                 uint64
	announced, unstable bool
}

// parseOwner reads "<pid>" and the optional ":a" / ":u" flags. A field that is set but unreadable is returned as
// present and not valid, so the process counts as a spawned child.
func parseOwner(text string) (o owner, present bool) {
	if text == "" {
		return owner{}, false
	}
	digits := text
	flags := ""
	if i := strings.IndexByte(text, ':'); i >= 0 {
		digits, flags = text[:i], text[i+1:]
	}
	pid, err := strconv.ParseUint(digits, 10, 64)
	if err != nil {
		return owner{}, true
	}
	o = owner{valid: true, pid: pid}
	for _, f := range flags {
		switch f {
		case 'a':
			o.announced = true
		case 'u':
			o.unstable = true
		default:
			o.valid = false
		}
	}
	return o, true
}

// ownerValue is the value of HAIRSPRING_PROC_PID for this process.
func ownerValue(pid int, announced, unstable bool) string {
	flags := ""
	if announced {
		flags += "a"
	}
	if unstable {
		flags += "u"
	}
	if flags == "" {
		return strconv.Itoa(pid)
	}
	return fmt.Sprintf("%d:%s", pid, flags)
}

// deriveTag is base + "." + letter + n, or "<first 40 bytes of base>.h<hash>" when that would not fit in a tag.
func deriveTag(base string, letter byte, n uint64) string {
	tag := fmt.Sprintf("%s.%c%d", base, letter, n)
	if len(tag) <= maxTagBytes {
		return tag
	}
	h := fnv.New64a()
	_, _ = h.Write([]byte(base))
	prefix := base[:min(len(base), hashPrefixBytes)]
	return fmt.Sprintf("%s.h%016x", prefix, h.Sum64()^(n*hashMix)^uint64(letter))
}
