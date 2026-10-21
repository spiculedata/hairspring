//! The call-site hot path. It does only in-memory counter updates: no system call, no allocation, no lock, no
//! printing, and the same instructions whether the condition holds or not (docs/rust-sdk.md has the argument).

use std::sync::atomic::Ordering::{Relaxed, Release};

use crate::site::{Site, Witness};
use crate::slots;

/// 1 when `x == 0`, else 0, without a branch.
#[inline(always)]
fn is_zero(x: u64) -> u64 {
    (((x | 0u64.wrapping_sub(x)) >> 63) ^ 1) & 1
}

/// Keep the compiler from turning the masks back into branches.
#[inline(always)]
fn opaque(v: u64) -> u64 {
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    {
        let mut v = v;
        // SAFETY: an empty assembly block that touches no memory and no stack.
        unsafe {
            std::arch::asm!("/* {0} */", inout(reg) v, options(nomem, nostack, preserves_flags));
        }
        v
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        std::hint::black_box(v)
    }
}

/// `take ? a : b` for references: an index into a two-element array, so no pointer is turned into an integer and
/// back. `take` is 0 or 1; the mask lets the compiler see the index is in bounds, so there is no check to branch on.
#[inline(always)]
fn select<'a>(a: &'a Witness, b: &'a Witness, take: u64) -> &'a Witness {
    [b, a][(take & 1) as usize]
}

/// Write a witness. Only one thread ever writes a given witness (the one whose counter left zero), and `ord` goes last
/// with release: a report that sees `ord` set, and reads it with acquire, sees every other field of the same write.
#[inline(always)]
pub(crate) fn store(w: &Witness, slot: u64, ord: u64, vals: &[i64; 4]) {
    w.slot.store(slot, Relaxed);
    w.vals[0].store(vals[0], Relaxed);
    w.vals[1].store(vals[1], Relaxed);
    w.vals[2].store(vals[2], Relaxed);
    w.vals[3].store(vals[3], Relaxed);
    w.ord.store(ord, Release);
}

/// Count one evaluation of a condition kind. The evaluation that moves a counter off zero owns that side's witness;
/// all others write to the thread's private scratch, so the stores happen either way.
#[inline(always)]
pub fn eval(site: &'static Site, ok: bool, vals: [i64; 4]) {
    let slot = slots::current();
    let ord = slot.evals.load(Relaxed) + 1;
    slot.evals.store(ord, Relaxed);
    let ok_mask = opaque(u64::from(ok));
    let bad = ok_mask ^ 1;
    let prev_true = site.trues.fetch_add(ok_mask, Relaxed);
    let prev_false = site.falses.fetch_add(bad, Relaxed);
    let index = slot.index.load(Relaxed);
    let win_true = ok_mask & is_zero(prev_true);
    let win_false = bad & is_zero(prev_false);
    store(
        select(&site.first_true, &slot.scratch, win_true),
        index,
        ord,
        &vals,
    );
    store(
        select(&site.first_false, &slot.scratch, win_false),
        index,
        ord,
        &vals,
    );
}

/// The same for kinds without a condition: always true, so only the true side exists.
#[inline(always)]
pub fn hit(site: &'static Site, vals: [i64; 4]) {
    let slot = slots::current();
    let ord = slot.evals.load(Relaxed) + 1;
    slot.evals.store(ord, Relaxed);
    let prev_true = site.trues.fetch_add(1, Relaxed);
    let index = slot.index.load(Relaxed);
    store(
        select(&site.first_true, &slot.scratch, is_zero(prev_true)),
        index,
        ord,
        &vals,
    );
}

/// Add `n` to this thread's work counter (reported as `work.ops`, summed over threads).
#[inline(always)]
pub fn work(n: u64) {
    let slot = slots::current();
    slot.work
        .store(slot.work.load(Relaxed).wrapping_add(n), Relaxed);
}
