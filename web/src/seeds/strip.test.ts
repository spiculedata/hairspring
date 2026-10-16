import { describe, expect, it } from 'vitest';
import { CH } from '../guest/chars';
import {
  appendStripPage,
  binStrip,
  countCodes,
  EMPTY_STRIP,
  keyToBin,
  MAX_STRIP_TRIALS,
  nextNotableBin,
  worstTrial,
} from './strip';

const repeat = (code: string, n: number) => code.repeat(n);

describe('appendStripPage', () => {
  it('appends consecutive pages and tracks the next expected sequence number', () => {
    const a = appendStripPage(EMPTY_STRIP, { first_seq: 0, codes: 'ppv' });
    const b = appendStripPage(a, { first_seq: 3, codes: 'pe' });
    expect(b.codes).toBe('ppvpe');
    expect(b.nextSeq).toBe(5);
    expect(b.truncated).toBe(false);
  });

  it('marks trials with no stored row as unknown rather than leaving them out or calling them passes', () => {
    const a = appendStripPage(EMPTY_STRIP, { first_seq: 0, codes: 'pp' });
    const b = appendStripPage(a, { first_seq: 5, codes: 'v' });
    expect(b.codes).toBe('pp---v');
    expect(b.nextSeq).toBe(6);
  });

  it('keeps a first page that starts after zero aligned to its sequence numbers', () => {
    expect(appendStripPage(EMPTY_STRIP, { first_seq: 2, codes: 'v' }).codes).toBe('--v');
  });

  it('ignores a page that overlaps what it already holds, so a repeated response cannot duplicate trials', () => {
    const a = appendStripPage(EMPTY_STRIP, { first_seq: 0, codes: 'ppv' });
    expect(appendStripPage(a, { first_seq: 1, codes: 'pv' })).toBe(a);
  });

  it('stops growing at the cap and says so, however large a page or gap a server sends', () => {
    const huge = appendStripPage(EMPTY_STRIP, { first_seq: MAX_STRIP_TRIALS + 5, codes: 'v' });
    expect(huge.codes.length).toBeLessThanOrEqual(MAX_STRIP_TRIALS);
    expect(huge.truncated).toBe(true);
    const full = appendStripPage(EMPTY_STRIP, { first_seq: 0, codes: repeat('p', MAX_STRIP_TRIALS) });
    expect(full.truncated).toBe(false);
    const more = appendStripPage(full, { first_seq: MAX_STRIP_TRIALS, codes: 'p' });
    expect(more.codes.length).toBe(MAX_STRIP_TRIALS);
    expect(more.truncated).toBe(true);
  });

  it('refuses codes it does not know: they become unknown, never a pass', () => {
    const codes = `p<z${CH.RLO}`;
    expect(appendStripPage(EMPTY_STRIP, { first_seq: 0, codes }).codes).toBe('p---');
  });
});

describe('binStrip', () => {
  it('gives every trial its own bin when there are fewer trials than pixels', () => {
    const bins = binStrip('ppvp', 100);
    expect(bins.map((b) => [b.start, b.end, b.worst])).toEqual([
      [0, 1, 'p'],
      [1, 2, 'p'],
      [2, 3, 'v'],
      [3, 4, 'p'],
    ]);
  });

  it('is empty for an empty run', () => {
    expect(binStrip('', 100)).toEqual([]);
    expect(binStrip('ppp', 0)).toEqual([]);
  });

  it('covers every trial exactly once, in order, with no more bins than pixels', () => {
    const codes = 'p'.repeat(1003);
    const bins = binStrip(codes, 100);
    expect(bins).toHaveLength(100);
    expect(bins[0]?.start).toBe(0);
    expect(bins[99]?.end).toBe(1003);
    for (let i = 1; i < bins.length; i++) expect(bins[i]?.start).toBe(bins[i - 1]?.end);
    for (const b of bins) expect(b.end).toBeGreaterThan(b.start);
  });

  it('never hides a violation inside a bin: the bin shows its worst trial', () => {
    const codes = repeat('p', 5000) + 'v' + repeat('p', 4999);
    const bins = binStrip(codes, 100);
    expect(bins.filter((b) => b.worst === 'v')).toHaveLength(1);
    const hit = bins.find((b) => b.worst === 'v');
    expect(hit && hit.start <= 5000 && 5000 < hit.end).toBe(true);
  });

  it('ranks inconclusive and excluded above pass, so they are never drawn as passes', () => {
    expect(binStrip('pppe', 1)[0]?.worst).toBe('e');
    expect(binStrip('pipp', 1)[0]?.worst).toBe('i');
    expect(binStrip('pvix', 1)[0]?.worst).toBe('v');
    expect(binStrip('piux', 1)[0]?.worst).toBe('x');
    expect(binStrip('p-pp', 1)[0]?.worst).toBe('-');
  });

  it('bins 100,000 trials in a single pass, quickly', () => {
    const codes = Array.from({ length: 100_000 }, (_, i) => (i % 997 === 0 ? 'v' : i % 101 === 0 ? 'e' : 'p')).join('');
    const t = performance.now();
    const bins = binStrip(codes, 1000);
    const ms = performance.now() - t;
    expect(bins).toHaveLength(1000);
    expect(bins.reduce((n, b) => n + (b.end - b.start), 0)).toBe(100_000);
    expect(bins.filter((b) => b.worst === 'v').length).toBeGreaterThan(90);
    expect(ms).toBeLessThan(250);
  });
});

describe('countCodes', () => {
  it('counts every class, including those with none', () => {
    expect(countCodes('ppvie-')).toEqual({ p: 2, v: 1, i: 1, u: 0, x: 0, e: 1, '-': 1 });
  });
});

describe('worstTrial', () => {
  it('is the first trial in the bin that has the bin worst class', () => {
    const codes = 'ppepvpv';
    const [bin] = binStrip(codes, 1);
    expect(bin && worstTrial(codes, bin)).toBe(4);
  });
});

describe('keyToBin', () => {
  it('moves one bin with the arrows, ten with the page keys, and to the ends with Home and End', () => {
    expect(keyToBin('ArrowRight', 5, 100)).toBe(6);
    expect(keyToBin('ArrowLeft', 5, 100)).toBe(4);
    expect(keyToBin('PageDown', 5, 100)).toBe(15);
    expect(keyToBin('PageUp', 5, 100)).toBe(0);
    expect(keyToBin('Home', 50, 100)).toBe(0);
    expect(keyToBin('End', 50, 100)).toBe(99);
  });

  it('stays inside the strip and ignores other keys', () => {
    expect(keyToBin('ArrowLeft', 0, 100)).toBe(0);
    expect(keyToBin('ArrowRight', 99, 100)).toBe(99);
    expect(keyToBin('a', 3, 100)).toBeUndefined();
    expect(keyToBin('ArrowRight', 0, 0)).toBeUndefined();
  });
});

describe('nextNotableBin', () => {
  const bins = binStrip('ppvpppeppp', 100);
  it('finds the next bin that is not a pass, forwards and backwards, and none past the ends', () => {
    expect(nextNotableBin(bins, 0, 1)).toBe(2);
    expect(nextNotableBin(bins, 2, 1)).toBe(6);
    expect(nextNotableBin(bins, 6, 1)).toBeUndefined();
    expect(nextNotableBin(bins, 6, -1)).toBe(2);
    expect(nextNotableBin(bins, 2, -1)).toBeUndefined();
  });
});
