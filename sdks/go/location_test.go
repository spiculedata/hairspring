package hairspring

import (
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

func TestDeclareAtRecordsTheCallingFileAndLine(t *testing.T) {
	c := NewCatalog()
	_, _, line, _ := runtime.Caller(0)

	h := declareAt(c, KindAlways, "loc.here", "where am I", 1)

	site := h.site
	if filepath.Base(site.File) != "location_test.go" || site.Line != uint64(line+2) {
		t.Fatalf("site location = %s:%d, want location_test.go:%d", site.File, site.Line, line+2)
	}
	if site.ID != "loc.here" || site.Kind != KindAlways || site.Msg != "where am I" {
		t.Fatalf("site = %+v", site)
	}
}

func TestEachKindHasAConstructorOnTheDefaultCatalog(t *testing.T) {
	saved := std
	std = NewCatalog()
	defer func() { std = saved }()

	handles := map[Kind]*Handle{
		KindAlways:              Always("k.always", "m"),
		KindAlwaysOrUnreachable: AlwaysOrUnreachable("k.aou", "m"),
		KindSometimes:           Sometimes("k.sometimes", "m"),
		KindReachable:           Reachable("k.reachable", "m"),
		KindUnreachable:         Unreachable("k.unreachable", "m"),
	}

	for kind, h := range handles {
		if h.site.Kind != kind || h.site.Line == 0 || !strings.HasSuffix(h.site.File, "location_test.go") {
			t.Errorf("%s constructor made %+v", kind, h.site)
		}
	}
}

func TestConstructorPanicsOnAnInvalidID(t *testing.T) {
	saved := std
	std = NewCatalog()
	defer func() { std = saved }()
	defer func() {
		if recover() == nil {
			t.Fatal("Always with an invalid id did not panic")
		}
	}()

	Always("not valid", "")
}

func TestCallerFileFallsBackToTheBaseNameWhenThePathIsTooLong(t *testing.T) {
	long := "/" + strings.Repeat("d/", 200) + "file.go"

	if got := fitFile(long); got != "file.go" {
		t.Fatalf("fitFile = %q, want the base name", got)
	}
	if got := fitFile("/src/app/main.go"); got != "/src/app/main.go" {
		t.Fatalf("fitFile = %q, want the path unchanged", got)
	}
	if got := fitFile("bad\x01name.go"); got != "" {
		t.Fatalf("fitFile = %q, want a file with a control character dropped", got)
	}
}
