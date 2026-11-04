import { describe, expect, it } from 'vitest';
import { parseThreadSlices, parseTimesliceStats } from './schedule';

describe('parseTimesliceStats', () => {
  it('reads the count and total from the summary text', () => {
    expect(parseTimesliceStats('{"count":3,"sum_ns":1500}')).toEqual({ count: 3, sumNs: 1500 });
  });

  it('gives undefined for anything that is not that shape, so the raw text is shown instead', () => {
    expect(parseTimesliceStats('nonsense')).toBeUndefined();
    expect(parseTimesliceStats('{"count":"x","sum_ns":1}')).toBeUndefined();
    expect(parseTimesliceStats('{"count":-1,"sum_ns":1}')).toBeUndefined();
    expect(parseTimesliceStats('[1,2]')).toBeUndefined();
  });
});

describe('parseThreadSlices', () => {
  it('turns an array of flat objects into columns and rows of text', () => {
    const parsed = parseThreadSlices('[{"thread":"a","count":2},{"thread":"b","count":5}]');
    expect(parsed).toEqual({
      columns: ['thread', 'count'],
      rows: [
        ['a', '2'],
        ['b', '5'],
      ],
    });
  });

  it('gives undefined for an empty array, nested values or text that is not JSON', () => {
    expect(parseThreadSlices('[]')).toBeUndefined();
    expect(parseThreadSlices('[{"a":{"b":1}}]')).toBeUndefined();
    expect(parseThreadSlices('not json')).toBeUndefined();
    expect(parseThreadSlices('{"a":1}')).toBeUndefined();
  });
});

describe('parseThreadSlices limits', () => {
  it('reads exactly the most rows and columns it allows', () => {
    const rows = Array.from({ length: 200 }, (_, i) => ({ n: i }));
    expect(parseThreadSlices(JSON.stringify(rows))?.rows).toHaveLength(200);
    const wide = Object.fromEntries(Array.from({ length: 8 }, (_, i) => [`c${i}`, i]));
    expect(parseThreadSlices(JSON.stringify([wide]))?.columns).toHaveLength(8);
  });

  it('falls back to raw text for 201 rows', () => {
    const rows = Array.from({ length: 201 }, (_, i) => ({ n: i }));
    expect(parseThreadSlices(JSON.stringify(rows))).toBeUndefined();
  });

  it('falls back to raw text for 9 columns', () => {
    const wide = Object.fromEntries(Array.from({ length: 9 }, (_, i) => [`c${i}`, i]));
    expect(parseThreadSlices(JSON.stringify([wide]))).toBeUndefined();
  });
});
