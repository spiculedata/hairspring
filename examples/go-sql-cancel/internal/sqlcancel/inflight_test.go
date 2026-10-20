package sqlcancel

import "testing"

func TestInflightFindsAnotherQueryButNeverTheCallersOwn(t *testing.T) {
	var f Inflight
	if _, ok := f.Other(5); ok {
		t.Fatal("nothing is in flight yet")
	}

	f.Enter(1, 5)
	if _, ok := f.Other(5); ok {
		t.Fatal("a query is not 'other' than itself")
	}
	if got, ok := f.Other(9); !ok || got != 5 {
		t.Fatalf("Other(9) = %d, %v; want 5, true", got, ok)
	}

	f.Leave(1)
	if _, ok := f.Other(9); ok {
		t.Fatal("left, so nothing in flight")
	}
}

func TestInflightTracksOneSlotPerWorker(t *testing.T) {
	var f Inflight
	f.Enter(0, 11)
	f.Enter(3, 12)
	f.Leave(0)
	if got, ok := f.Other(1); !ok || got != 12 {
		t.Fatalf("Other = %d, %v; want 12, true", got, ok)
	}
}
