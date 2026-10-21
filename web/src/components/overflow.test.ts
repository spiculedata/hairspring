import { describe, expect, it } from 'vitest';
import { isOverflowing } from './overflow';

describe('isOverflowing', () => {
  it('is false when the content fits', () => {
    expect(isOverflowing(968, 968)).toBe(false);
    expect(isOverflowing(900, 968)).toBe(false);
  });

  it('is true when the content is wider than the box, even by a few pixels (a 1000 to 1015 px window with a scrollbar)', () => {
    expect(isOverflowing(968, 963)).toBe(true);
    expect(isOverflowing(968, 940)).toBe(true);
  });

  it('ignores the sub-pixel rounding the browser reports for a table that fits', () => {
    expect(isOverflowing(968, 967.6)).toBe(false);
  });
});
