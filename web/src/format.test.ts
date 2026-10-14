import { describe, expect, it } from 'vitest';
import { formatDuration, formatSeedInput, relativeTime, stateTone } from './format';

describe('relativeTime', () => {
  const now = 1_000_000_000_000;
  it('describes recent and older instants', () => {
    expect(relativeTime(now - 4_000, now)).toBe('4s ago');
    expect(relativeTime(now - 90_000, now)).toBe('1m ago');
    expect(relativeTime(now - 3 * 3_600_000, now)).toBe('3h ago');
    expect(relativeTime(now - 2 * 86_400_000, now)).toBe('2d ago');
  });
  it('never shows negative time for clock skew', () => {
    expect(relativeTime(now + 5_000, now)).toBe('just now');
  });
});

describe('formatDuration', () => {
  it('formats milliseconds compactly', () => {
    expect(formatDuration(0)).toBe('0 ms');
    expect(formatDuration(850)).toBe('850 ms');
    expect(formatDuration(12_340)).toBe('12.3 s');
    expect(formatDuration(125_000)).toBe('2m 5s');
  });
});

describe('stateTone', () => {
  it('maps run states to display tones', () => {
    expect(stateTone('passed')).toBe('good');
    expect(stateTone('failed')).toBe('bad');
    expect(stateTone('error')).toBe('bad');
    expect(stateTone('running')).toBe('busy');
    expect(stateTone('queued')).toBe('idle');
    expect(stateTone('cancelled')).toBe('idle');
  });
});

describe('formatSeedInput', () => {
  it('accepts blank (random) and safe integers', () => {
    expect(formatSeedInput('')).toEqual({ ok: true, value: undefined });
    expect(formatSeedInput(' 42 ')).toEqual({ ok: true, value: 42 });
  });
  it('rejects values JavaScript cannot represent exactly', () => {
    expect(formatSeedInput('9007199254740993').ok).toBe(false);
    expect(formatSeedInput('-1').ok).toBe(false);
    expect(formatSeedInput('1.5').ok).toBe(false);
    expect(formatSeedInput('abc').ok).toBe(false);
  });
});
