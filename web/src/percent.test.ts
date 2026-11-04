import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { lowerPct, nearestPct, upperPct } from './percent';

const CASES_FILE = new URL('../../crates/hairspring-props/tests/percent_cases.txt', import.meta.url);
const formatters = { upper: upperPct, lower: lowerPct, nearest: nearestPct } as const;

interface Case {
  readonly direction: keyof typeof formatters;
  readonly fraction: number;
  readonly expected: string;
  readonly line: string;
}

function readCases(): Case[] {
  return readFileSync(CASES_FILE, 'utf8')
    .split('\n')
    .filter((l) => l.trim() !== '' && !l.startsWith('#'))
    .map((line) => {
      const [direction, fraction, expected] = line.trim().split(/\s+/);
      if (!(direction in formatters)) throw new Error(`unknown direction in ${line}`);
      return { direction: direction as Case['direction'], fraction: Number(fraction), expected, line };
    });
}

describe('directed rounding, against the CLI report cases', () => {
  const cases = readCases();

  it('reads the cases the Rust tests read', () => {
    expect(cases.length).toBeGreaterThanOrEqual(15);
  });

  it.each(cases)('$line', ({ direction, fraction, expected }) => {
    expect(formatters[direction](fraction)).toBe(expected);
  });
});

describe('directed rounding, out of range', () => {
  it('clamps to 0..100%', () => {
    expect(upperPct(1.2)).toBe('100.00%');
    expect(lowerPct(-1)).toBe('0.00%');
  });

  it('treats NaN as the value that overstates least, as the CLI does', () => {
    expect(upperPct(Number.NaN)).toBe('100.00%');
    expect(lowerPct(Number.NaN)).toBe('0.00%');
    expect(nearestPct(Number.NaN)).toBe('0.00%');
  });
});
