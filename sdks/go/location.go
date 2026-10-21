package hairspring

import (
	"path"
	"runtime"
)

// Always declares an always site on the process-wide catalog, recording the file and line of the caller. Call it from
// a package-level var or init, before Main. An invalid id is a programming error and panics at startup.
func Always(id, msg string) *Handle { return declareAt(std, KindAlways, id, msg, 2) }

// AlwaysOrUnreachable declares an always_or_unreachable site; see Always.
func AlwaysOrUnreachable(id, msg string) *Handle {
	return declareAt(std, KindAlwaysOrUnreachable, id, msg, 2)
}

// Sometimes declares a sometimes site; see Always.
func Sometimes(id, msg string) *Handle { return declareAt(std, KindSometimes, id, msg, 2) }

// Reachable declares a reachable site; see Always. Evaluate it with Handle.Hit.
func Reachable(id, msg string) *Handle { return declareAt(std, KindReachable, id, msg, 2) }

// Unreachable declares an unreachable site; see Always. Evaluate it with Handle.Hit.
func Unreachable(id, msg string) *Handle { return declareAt(std, KindUnreachable, id, msg, 2) }

// declareAt declares a site located at the caller `skip` frames up (1 is the caller of declareAt).
func declareAt(c *Catalog, kind Kind, id, msg string, skip int) *Handle {
	_, file, line, ok := runtime.Caller(skip)
	site := Site{ID: id, Kind: kind, Msg: msg}
	if ok {
		site.File, site.Line = fitFile(file), uint64(line)
	}
	return declareOrPanic(c, site)
}

// fitFile makes a source path acceptable to the protocol: the path itself when it fits, else its base name, else
// nothing (a path with a control character, which the protocol forbids).
func fitFile(file string) string {
	if checkText(file, maxFileBytes) == nil {
		return file
	}
	if base := path.Base(file); base != file && checkText(base, maxFileBytes) == nil {
		return base
	}
	return ""
}
