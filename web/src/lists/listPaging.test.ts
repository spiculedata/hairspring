import { describe, expect, it } from 'vitest';
import { applyHead, applyMore, rowsOf, type Held, type ListPage } from './listPaging';

interface Row {
  id: string;
  state?: string;
}
const id = (r: Row): string => r.id;
const row = (n: number, state = 'queued'): Row => ({ id: `r${n}`, state });
const page = (ns: number[], nextCursor?: string): ListPage<Row> => ({ items: ns.map((n) => row(n)), nextCursor });
const ids = (held: Held<Row>): string[] => rowsOf(held, id).map((r) => r.id);

describe('applyHead', () => {
  it('starts a list from the first page, with its cursor', () => {
    const held = applyHead(undefined, page([9, 8, 7], '3'), id);
    expect(ids(held)).toEqual(['r9', 'r8', 'r7']);
    expect(held.cursor).toBe('3');
    expect(held.epoch).toBe(0);
  });

  it('replaces the rows and the cursor when nothing more has been loaded', () => {
    const first = applyHead(undefined, page([9, 8, 7], '3'), id);
    const next = applyHead(first, page([10, 9, 8], '3'), id);
    expect(ids(next)).toEqual(['r10', 'r9', 'r8']);
    expect(next.cursor).toBe('3');
  });

  it('shows a changed row in the new state', () => {
    const first = applyHead(undefined, { items: [row(2, 'running'), row(1)], nextCursor: undefined }, id);
    const next = applyHead(first, { items: [row(2, 'failed'), row(1)], nextCursor: undefined }, id);
    expect(rowsOf(next, id)[0]).toEqual({ id: 'r2', state: 'failed' });
  });

  it('keeps the pages loaded after the first and their cursor when the refresh shares a row with what is held', () => {
    let held = applyHead(undefined, page([9, 8, 7], '3'), id);
    held = applyMore(held, page([6, 5, 4], '6'), 0, id) as Held<Row>;
    const refreshed = applyHead(held, page([10, 9, 8], '3'), id);
    expect(refreshed.cursor).toBe('6');
    expect(refreshed.epoch).toBe(0);
  });

  it('keeps a row that a newer row pushed out of the first page, so no row goes missing between the pages', () => {
    let held = applyHead(undefined, page([9, 8, 7], '3'), id);
    held = applyMore(held, page([6, 5, 4], '6'), 0, id) as Held<Row>;
    const refreshed = applyHead(held, page([10, 9, 8], '3'), id);
    expect(ids(refreshed)).toEqual(['r10', 'r9', 'r8', 'r7', 'r6', 'r5', 'r4']);
    const again = applyHead(refreshed, page([11, 10, 9], '3'), id);
    expect(ids(again)).toEqual(['r11', 'r10', 'r9', 'r8', 'r7', 'r6', 'r5', 'r4']);
  });

  it('shows a row that moved from a loaded page into the first page once', () => {
    let held = applyHead(undefined, page([9, 8, 7], '3'), id);
    held = applyMore(held, page([6, 5, 4]), 0, id) as Held<Row>;
    const refreshed = applyHead(held, page([6, 9, 8], '3'), id);
    expect(ids(refreshed).filter((x) => x === 'r6')).toHaveLength(1);
  });

  it('starts again from the first page when the refresh shares no row, because rows between would be missing', () => {
    let held = applyHead(undefined, page([9, 8, 7], '3'), id);
    held = applyMore(held, page([6, 5, 4], '6'), 0, id) as Held<Row>;
    const restarted = applyHead(held, page([30, 29, 28], '3'), id);
    expect(ids(restarted)).toEqual(['r30', 'r29', 'r28']);
    expect(restarted.cursor).toBe('3');
    expect(restarted.epoch).toBe(1);
  });

  it('does not change what it was given', () => {
    const held = applyHead(undefined, page([9, 8], '2'), id);
    const frozen = JSON.stringify(held);
    applyHead(held, page([10, 9], '2'), id);
    applyMore(held, page([7], undefined), 0, id);
    expect(JSON.stringify(held)).toBe(frozen);
  });
});

describe('applyMore', () => {
  it('appends the new rows after those held and takes the new cursor', () => {
    const held = applyHead(undefined, page([9, 8, 7], '3'), id);
    const more = applyMore(held, page([6, 5], '5'), 0, id) as Held<Row>;
    expect(ids(more)).toEqual(['r9', 'r8', 'r7', 'r6', 'r5']);
    expect(more.cursor).toBe('5');
  });

  it('drops rows that were already held, as when a new row moved the pages down', () => {
    const held = applyHead(undefined, page([9, 8, 7], '3'), id);
    const more = applyMore(held, page([7, 6, 5], '6'), 0, id) as Held<Row>;
    expect(ids(more)).toEqual(['r9', 'r8', 'r7', 'r6', 'r5']);
    expect(more.extra.map(id)).toEqual(['r6', 'r5']);
  });

  it('has no cursor once the last page arrives', () => {
    const held = applyHead(undefined, page([3, 2], '2'), id);
    expect((applyMore(held, page([1]), 0, id) as Held<Row>).cursor).toBeUndefined();
  });

  it('ignores a page requested before the list started again', () => {
    const held = applyHead(undefined, page([9], '1'), id);
    const restarted = applyHead(applyMore(held, page([8], '2'), 0, id), page([40], '1'), id);
    expect(restarted.epoch).toBe(1);
    expect(applyMore(restarted, page([5, 4], '4'), 0, id)).toBe(restarted);
  });

  it('ignores a page when there is no list', () => {
    expect(applyMore(undefined, page([1]), 0, id)).toBeUndefined();
  });
});
