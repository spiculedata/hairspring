import { describe, expect, it } from 'vitest';
import { OUTPUT_MAX_CHARS, OUTPUT_MAX_LINES, prepareTail } from './output';

describe('prepareTail', () => {
  it('keeps a short output whole, one entry per line, without the empty piece after the last newline', () => {
    expect(prepareTail('a\nb\n')).toEqual({ lines: ['a', 'b'], charsDropped: 0, linesDropped: 0 });
    expect(prepareTail('no newline')).toEqual({ lines: ['no newline'], charsDropped: 0, linesDropped: 0 });
    expect(prepareTail('')).toEqual({ lines: [], charsDropped: 0, linesDropped: 0 });
  });

  it('keeps the end of a large output, counts what it dropped, and never holds more than the cap', () => {
    const big = 'x'.repeat(10 * 1024 * 1024);
    const t = prepareTail(big);
    expect(t.lines.join('').length).toBeLessThanOrEqual(OUTPUT_MAX_CHARS);
    expect(t.charsDropped).toBe(big.length - OUTPUT_MAX_CHARS);
  });

  it('keeps the last lines when there are more than it shows, and says how many it left out', () => {
    const many = Array.from({ length: OUTPUT_MAX_LINES + 50 }, (_, i) => `l${i}`).join('\n');
    const t = prepareTail(many);
    expect(t.lines).toHaveLength(OUTPUT_MAX_LINES);
    expect(t.lines.at(-1)).toBe(`l${OUTPUT_MAX_LINES + 49}`);
    expect(t.linesDropped).toBe(50);
  });

  it('does not start in the middle of a surrogate pair', () => {
    const text = `${'a'.repeat(OUTPUT_MAX_CHARS - 1)}${String.fromCodePoint(0x1f600)}`;
    const t = prepareTail(`${'b'}${text}`);
    expect(t.lines.join('')).not.toMatch(/^[\udc00-\udfff]/);
  });
});
