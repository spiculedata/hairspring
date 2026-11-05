import { failurePath, runPath, trialPath } from '../router';
import type { SearchResults } from './searchApi';

export type GroupId = 'runs' | 'failures' | 'targets' | 'properties' | 'seeds' | 'schedules';

/** One selectable hit. The words the page writes are `main` and `detail`; text from the program under test is `guestText`, shown only through `GuestText`. */
export interface HitRow {
  /** Position in the whole list, across groups: what the keyboard moves through. */
  readonly index: number;
  readonly key: string;
  readonly path: string;
  readonly main?: string;
  readonly guestText?: string;
  readonly detail: string;
}

export interface HitGroup {
  readonly id: GroupId;
  readonly title: string;
  readonly rows: readonly HitRow[];
  /** The server has more hits than it listed. */
  readonly more: boolean;
}

type Draft = Omit<HitRow, 'index'>;

const TITLES: Readonly<Record<GroupId, string>> = {
  runs: 'Runs',
  failures: 'Failures',
  targets: 'Targets',
  properties: 'Properties',
  seeds: 'Seeds',
  schedules: 'Schedules',
};

const ORDER: readonly GroupId[] = ['runs', 'failures', 'targets', 'properties', 'seeds', 'schedules'];

const present = (path: string | undefined, draft: Omit<Draft, 'path'>): Draft[] => (path === undefined ? [] : [{ ...draft, path }]);

/** A run's address, or nothing when the id is not a plain one (`runPath` answers '/' for those). */
const runAddress = (id: string): string | undefined => {
  const path = runPath(id);
  return path === '/' ? undefined : path;
};

function draftsOf(id: GroupId, r: SearchResults): Draft[] {
  switch (id) {
    case 'runs':
      return r.runs.flatMap((h) => present(runAddress(h.id), { key: h.id, main: h.id, detail: h.target }));
    case 'failures':
      return r.failures.flatMap((h) => present(failurePath(h.id), { key: h.id, main: h.id, detail: `Run ${h.run_id}` }));
    case 'targets':
      return r.targets.map((h) => ({ key: h.name, path: `/runs?target=${encodeURIComponent(h.name)}`, main: h.name, detail: 'Runs of this target' }));
    case 'properties':
      // The key is built from the position, so no text from the program under test sits in a field the page treats as its own.
      return r.properties.flatMap((h, i) => present(runAddress(h.run_id), { key: `property-${i}`, guestText: h.guest.id, detail: `Newest run with it: ${h.run_id}` }));
    case 'seeds':
      return r.seeds.flatMap((h) =>
        h.trial === null
          ? present(runAddress(h.run_id), { key: `${h.run_id}/run`, main: String(h.seed), detail: `Run ${h.run_id} started with it` })
          : present(trialPath(h.run_id, h.trial), { key: `${h.run_id}/${h.trial}`, main: String(h.seed), detail: `Trial ${h.trial} of ${h.run_id}` }),
      );
    case 'schedules':
      return r.schedules.flatMap((h) => present(trialPath(h.run_id, h.trial), { key: `${h.run_id}/${h.trial}`, main: h.fingerprint, detail: `Trial ${h.trial} of ${h.run_id}` }));
  }
}

/** The answer as the groups to show: only groups with hits, each row numbered across the whole list. */
export function groupHits(results: SearchResults): HitGroup[] {
  let next = 0;
  return ORDER.flatMap((id) => {
    const drafts = draftsOf(id, results);
    if (drafts.length === 0) return [];
    const rows = drafts.map((d) => ({ ...d, index: next++ }));
    return [{ id, title: TITLES[id], rows, more: results.more.includes(id) }];
  });
}

export const hitCount = (groups: readonly HitGroup[]): number => groups.reduce((n, g) => n + g.rows.length, 0);

/** How far the search looked. An empty answer means "none in this much", not "never stored". */
export function scopeNote(scope: SearchResults['scope']): string {
  return `Looked in the ${scope.recent_runs} newest runs (the ${scope.property_runs} newest runs for property ids), and for a whole run or failure id of any age. Targets are matched by the start of their name.`;
}
