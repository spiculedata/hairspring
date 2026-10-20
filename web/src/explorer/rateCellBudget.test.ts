import { describe, expect, it } from 'vitest';
import { CLIP_HEIGHT, RATE_CELL_BLOCKS_PX, RATE_CELL_WORST_PX } from './PropertyRowView';

describe('property row height', () => {
  it('has room for the tallest rate cell: figure, two-line qualifier, bar, UNMET note and inconclusive note', () => {
    const sum = Object.values(RATE_CELL_BLOCKS_PX).reduce((a, b) => a + b, 0);
    expect(sum).toBe(RATE_CELL_WORST_PX);
    expect(CLIP_HEIGHT).toBeGreaterThanOrEqual(RATE_CELL_WORST_PX);
  });
});
