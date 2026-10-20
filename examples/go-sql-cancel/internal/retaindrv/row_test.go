package retaindrv

import "testing"

func TestAppendRowFormatsTheQueryIdTwiceAroundTheRowIndex(t *testing.T) {
	// Arrange / Act
	got := string(AppendRow(nil, 7, 0))

	// Assert
	if want := "q00000007-r0000-q00000007"; got != want {
		t.Fatalf("AppendRow(7,0) = %q, want %q", got, want)
	}
	if len(got) != RowLen {
		t.Fatalf("row is %d bytes, want %d", len(got), RowLen)
	}
}

func TestAppendRowAppendsToTheGivenSlice(t *testing.T) {
	got := string(AppendRow([]byte("x"), 12, 3))
	if want := "xq00000012-r0003-q00000012"; got != want {
		t.Fatalf("got %q, want %q", got, want)
	}
}

func TestAppendRowDoesNotAllocateIntoAPresizedBuffer(t *testing.T) {
	var buf [RowLen]byte
	allocs := testing.AllocsPerRun(100, func() { _ = AppendRow(buf[:0], 4000, 1) })
	if allocs != 0 {
		t.Fatalf("AppendRow allocated %v times", allocs)
	}
}

func TestAppendRowPanicsOutsideTheFormatRange(t *testing.T) {
	for _, c := range []struct {
		name string
		qid  int64
		row  int
	}{
		{"negative id", -1, 0},
		{"id too wide", MaxQueryID + 1, 0},
		{"negative row", 1, -1},
		{"row too wide", 1, MaxRowIndex + 1},
	} {
		t.Run(c.name, func(t *testing.T) {
			defer func() {
				if recover() == nil {
					t.Fatal("expected a panic")
				}
			}()
			AppendRow(nil, c.qid, c.row)
		})
	}
}

func TestParseRowRoundTrips(t *testing.T) {
	for _, qid := range []int64{1, 7, 4000, MaxQueryID} {
		r, ok := ParseRow(AppendRow(nil, qid, 9))
		if !ok || r != (Row{Head: qid, Index: 9, Tail: qid}) {
			t.Fatalf("qid %d: got %+v ok=%v", qid, r, ok)
		}
	}
}

func TestParseRowReadsATornRow(t *testing.T) {
	torn := append(AppendRow(nil, 1, 0)[:12], AppendRow(nil, 2, 0)[12:]...)
	r, ok := ParseRow(torn)
	if !ok || r.Head != 1 || r.Tail != 2 {
		t.Fatalf("got %+v ok=%v", r, ok)
	}
}

func TestParseRowRejectsMalformedInput(t *testing.T) {
	good := string(AppendRow(nil, 5, 0))
	for name, in := range map[string]string{
		"empty":         "",
		"short":         good[:24],
		"long":          good + "x",
		"bad head tag":  "x" + good[1:],
		"bad separator": good[:9] + "_" + good[10:],
		"letter digit":  good[:3] + "x" + good[4:],
		"bad tail tag":  good[:17] + "z" + good[18:],
		"tail letter":   good[:24] + "x",
	} {
		if r, ok := ParseRow([]byte(in)); ok {
			t.Errorf("%s: accepted %q as %+v", name, in, r)
		}
	}
}
