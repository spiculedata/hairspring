package hairspring;

import java.util.List;

/** The table of per-thread slots. */
final class SlotsTests {
    private SlotsTests() {}

    static void register(Suite s) {
        s.add("slots: a thread claims claim-order keys t#1, t#2 ...", () -> {
            SlotTable table = new SlotTable();
            Check.equal("t#1", table.claim().slot().key);
            Check.equal("t#2", table.claim().slot().key);
            Check.equal(2, table.used());
        });
        s.add("slots: 300 claims fill 255 slots and one shared overflow slot", () -> {
            SlotTable table = new SlotTable();
            int shared = 0;
            for (int i = 0; i < 300; i++) {
                if (table.claim().shared()) {
                    shared++;
                }
            }
            Check.equal(256, table.used());
            Check.equal(45, shared);
            Check.equal("t#overflow", table.keyAt(255));
            Check.equal("t#overflow", table.keyAt(1000));
        });
        s.add("slots: thread() gives a stable key label-index", () -> {
            SlotTable table = new SlotTable();
            Slot slot = table.claim().slot();
            Check.yes(table.setKey(slot, "worker", 3), "claimed");
            Check.equal("worker-3", slot.key);
            Check.yes(slot.stable, "stable");
            Check.equal("t#1", new SlotTable().claim().slot().key);
        });
        s.add("slots: bytes of the label outside the alphabet become underscores", () -> {
            SlotTable table = new SlotTable();
            Slot slot = table.claim().slot();
            table.setKey(slot, "a b", 1);
            Check.equal("a_b-1", slot.key);
        });
        s.add("slots: a key another thread has is refused and the claim-order key stays", () -> {
            SlotTable table = new SlotTable();
            Slot first = table.claim().slot();
            Slot second = table.claim().slot();
            Check.yes(table.setKey(first, "w", 0), "first");
            Check.no(table.setKey(second, "w", 0), "second");
            Check.equal("t#2", second.key);
            Check.no(second.stable, "stable");
        });
        s.add("slots: a thread that shares the overflow slot cannot name itself", () -> {
            SlotTable table = new SlotTable();
            Slot last = null;
            for (int i = 0; i < 256; i++) {
                last = table.claim().slot();
            }
            Check.no(table.setKey(last, "late", 0), "overflow slot");
        });
        s.add("slots: an empty label, a bad index or a key over 64 bytes is an error", () -> {
            SlotTable table = new SlotTable();
            Slot slot = table.claim().slot();
            Check.raises(IllegalArgumentException.class, () -> table.setKey(slot, "", 0));
            Check.raises(IllegalArgumentException.class, () -> table.setKey(slot, "a", -1));
            Check.raises(IllegalArgumentException.class, () -> table.setKey(slot, "a", 4294967296L));
            Check.raises(IllegalArgumentException.class, () -> table.setKey(slot, "k".repeat(70), 0));
            Check.raises(NullPointerException.class, () -> table.setKey(slot, null, 0));
        });
        s.add("slots: views are immutable copies", () -> {
            SlotTable table = new SlotTable();
            Slot slot = table.claim().slot();
            slot.evals = 5;
            slot.work = 9;
            List<ThreadView> views = table.views();
            slot.evals = 6;
            Check.equal(List.of(new ThreadView("t#1", 5, 9, 0, false)), views);
        });
    }
}
