/**
 * How a newest-first list that is refreshed from its first page and extended page by page keeps its rows.
 *
 * The server pages by position, so a row stored while a reader pages moves later rows down by one. The rules here keep
 * every row once and, where the list only grows at its newest end, leave none out:
 * - rows are told apart by id, and a row that arrives again (the page moved under the reader) is kept once;
 * - a refresh of the first page replaces the first rows (so a run that has finished shows its new state) and keeps the
 *   pages loaded after it, with the cursor those pages reached (even when the last one added no row, as when a state
 *   filter found no match in the stretch it read), as long as the refresh shares at least one row with what is held;
 *   when it shares none, more than a page of newer rows arrived and the rows between would be missing, so the loaded
 *   pages are dropped and the list starts again from the first page;
 * - the rows a newer row pushed out of the first page are kept with the older rows, unless `rowsLeave` says a row can
 *   leave the list (a list filtered by a state that changes): a pushed-out row may have left it, and keeping it would
 *   show it with data that is out of date. Such a row is dropped and comes back if the next page still holds it;
 * - rows loaded by asking for more are shown as they were when loaded, and are not refreshed until the list starts
 *   again. Where rows can leave a list, a row can also be passed over by the next page, because the cursor is a
 *   position;
 * - a page that was requested before a restart is ignored when it arrives (`epoch`).
 *
 * All functions return new values and change nothing they are given.
 */

/** One page as the server sent it. */
export interface ListPage<T> {
  readonly items: readonly T[];
  /** Where the next page starts; absent on the last page. */
  readonly nextCursor: string | undefined;
}

export interface Held<T> {
  /** The newest rows, as last refreshed. */
  readonly head: readonly T[];
  /** Rows loaded after the head by asking for more, oldest last. */
  readonly extra: readonly T[];
  /** Where the page after what is held starts; undefined when nothing older remains. */
  readonly cursor: string | undefined;
  /** Counts restarts; a page requested under an earlier count is ignored. */
  readonly epoch: number;
  /** A further page has been added since the list started, so `cursor` may be ahead of the first page's own. */
  readonly moved: boolean;
}

export interface HeadOptions {
  /** Rows can leave the list between loads (a filter on a state that changes), so a row the first page no longer holds is dropped. */
  readonly rowsLeave?: boolean;
}

const idsOf = <T>(rows: readonly T[], id: (row: T) => string): Set<string> => new Set(rows.map(id));

/** Every held row once, newest first. */
export function rowsOf<T>(held: Held<T>, id: (row: T) => string): readonly T[] {
  const inHead = idsOf(held.head, id);
  return [...held.head, ...held.extra.filter((row) => !inHead.has(id(row)))];
}

/** The state after the first page was loaded again. */
export function applyHead<T>(prev: Held<T> | undefined, page: ListPage<T>, id: (row: T) => string, options: HeadOptions = {}): Held<T> {
  if (prev === undefined) return { head: page.items, extra: [], cursor: page.nextCursor, epoch: 0, moved: false };
  if (!prev.moved) return { ...prev, head: page.items, cursor: page.nextCursor };
  const known = idsOf([...prev.head, ...prev.extra], id);
  const sharesARow = page.items.some((row) => known.has(id(row)));
  if (page.items.length > 0 && known.size > 0 && !sharesARow) {
    return { head: page.items, extra: [], cursor: page.nextCursor, epoch: prev.epoch + 1, moved: false };
  }
  // Rows the new first page pushed out are older than it and newer than what was loaded after it: they join the older rows.
  const inNewHead = idsOf(page.items, id);
  const pushedOut = options.rowsLeave === true ? [] : prev.head.filter((row) => !inNewHead.has(id(row)));
  return { ...prev, head: page.items, extra: [...pushedOut, ...prev.extra] };
}

/** The state after a further page that was requested under `epoch` arrived. A page from before a restart changes nothing. */
export function applyMore<T>(prev: Held<T> | undefined, page: ListPage<T>, epoch: number, id: (row: T) => string): Held<T> | undefined {
  if (prev === undefined || prev.epoch !== epoch) return prev;
  const known = idsOf([...prev.head, ...prev.extra], id);
  return { ...prev, extra: [...prev.extra, ...page.items.filter((row) => !known.has(id(row)))], cursor: page.nextCursor, moved: true };
}
