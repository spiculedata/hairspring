namespace Hairspring;

/// <summary>The table of per-thread slots.</summary>
internal static class SlotsTests
{
    public static void Register(Suite s)
    {
        s.Add("slots: a thread claims claim-order keys t#1, t#2 ...", () =>
        {
            var table = new SlotTable();
            Check.Equal("t#1", table.Claim().Slot.Key);
            Check.Equal("t#2", table.Claim().Slot.Key);
            Check.Equal(2, table.Used);
        });
        s.Add("slots: 300 claims fill 255 slots and one shared overflow slot", () =>
        {
            var table = new SlotTable();
            var shared = 0;
            for (var i = 0; i < 300; i++)
            {
                if (table.Claim().Shared)
                {
                    shared++;
                }
            }
            Check.Equal(256, table.Used);
            Check.Equal(45, shared);
            Check.Equal("t#overflow", table.KeyAt(255));
            Check.Equal("t#overflow", table.KeyAt(1000));
        });
        s.Add("slots: NameThread gives a stable key label-index", () =>
        {
            var table = new SlotTable();
            var slot = table.Claim().Slot;
            Check.Yes(table.SetKey(slot, "worker", 3), "claimed");
            Check.Equal("worker-3", slot.Key);
            Check.Yes(slot.Stable, "stable");
            Check.Equal("t#1", new SlotTable().Claim().Slot.Key);
        });
        s.Add("slots: bytes of the label outside the alphabet become underscores", () =>
        {
            var table = new SlotTable();
            var slot = table.Claim().Slot;
            table.SetKey(slot, "a b", 1);
            Check.Equal("a_b-1", slot.Key);
        });
        s.Add("slots: a key another thread has is refused and the claim-order key stays", () =>
        {
            var table = new SlotTable();
            var first = table.Claim().Slot;
            var second = table.Claim().Slot;
            Check.Yes(table.SetKey(first, "w", 0), "first");
            Check.No(table.SetKey(second, "w", 0), "second");
            Check.Equal("t#2", second.Key);
            Check.No(second.Stable, "stable");
        });
        s.Add("slots: a thread that shares the overflow slot cannot name itself", () =>
        {
            var table = new SlotTable();
            Slot? last = null;
            for (var i = 0; i < 256; i++)
            {
                last = table.Claim().Slot;
            }
            Check.No(table.SetKey(last!, "late", 0), "overflow slot");
        });
        s.Add("slots: an empty label, a bad index or a key over 64 bytes is an error", () =>
        {
            var table = new SlotTable();
            var slot = table.Claim().Slot;
            Check.Raises<System.ArgumentException>(() => table.SetKey(slot, "", 0));
            Check.Raises<System.ArgumentOutOfRangeException>(() => table.SetKey(slot, "a", -1));
            Check.Raises<System.ArgumentOutOfRangeException>(() => table.SetKey(slot, "a", 4294967296L));
            Check.Raises<System.ArgumentException>(() => table.SetKey(slot, new string('k', 70), 0));
            Check.Raises<System.ArgumentNullException>(() => table.SetKey(slot, null!, 0));
        });
        s.Add("slots: views are immutable copies", () =>
        {
            var table = new SlotTable();
            var slot = table.Claim().Slot;
            slot.Evals = 5;
            slot.Work = 9;
            var views = table.Views();
            slot.Evals = 6;
            Check.Equal(new[] { new ThreadView("t#1", 5, 9, 0, false) }, views);
        });
    }
}
