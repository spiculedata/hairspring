import { describe, expect, it } from 'vitest';
import { groupHits, hitCount, scopeNote } from './searchHits';
import type { SearchResults } from './searchApi';

const EMPTY: SearchResults = {
  runs: [],
  failures: [],
  targets: [],
  properties: [],
  seeds: [],
  schedules: [],
  more: [],
  scope: { per_group: 5, recent_runs: 1000, property_runs: 100 },
};

const RUN = 'run_8f3a9c0d11b2e4a7';

describe('groupHits', () => {
  it('lists only the groups that have hits, in a fixed order, each row with where it goes', () => {
    const groups = groupHits({
      ...EMPTY,
      runs: [{ id: RUN, target: 'lock-lease', state: 'failed' }],
      failures: [{ id: 'fail_00112233445566ff', run_id: RUN, target: 'lock-lease' }],
      targets: [{ name: 'lock-lease' }],
      properties: [{ run_id: RUN, guest: { note: 'n', id: 'db.commit' } }],
      seeds: [{ run_id: RUN, seed: 777001, trial: 0 }, { run_id: RUN, seed: 424242, trial: null }],
      schedules: [{ fingerprint: '0123456789abcdef', run_id: RUN, trial: 3 }],
    });
    expect(groups.map((g) => g.id)).toEqual(['runs', 'failures', 'targets', 'properties', 'seeds', 'schedules']);
    expect(groups.map((g) => g.rows.map((r) => r.path))).toEqual([
      [`/runs/${RUN}`],
      ['/failures/fail_00112233445566ff'],
      ['/runs?target=lock-lease'],
      [`/runs/${RUN}`],
      [`/runs/${RUN}/trials/0`, `/runs/${RUN}`],
      [`/runs/${RUN}/trials/3`],
    ]);
  });

  it('leaves out a group with no hits', () => {
    expect(groupHits(EMPTY)).toEqual([]);
    expect(groupHits({ ...EMPTY, targets: [{ name: 'a' }] }).map((g) => g.id)).toEqual(['targets']);
  });

  it('numbers the rows across groups, so the keyboard can walk them as one list', () => {
    const groups = groupHits({ ...EMPTY, runs: [{ id: RUN, target: 't', state: 'failed' }, { id: 'run_0000000000000001', target: 't', state: 'failed' }], targets: [{ name: 'a' }] });
    expect(groups.flatMap((g) => g.rows.map((r) => r.index))).toEqual([0, 1, 2]);
    expect(hitCount(groups)).toBe(3);
  });

  it('marks a group that the server says has more hits', () => {
    const groups = groupHits({ ...EMPTY, runs: [{ id: RUN, target: 't', state: 'failed' }], targets: [{ name: 'a' }], more: ['runs'] });
    expect(groups.map((g) => [g.id, g.more])).toEqual([['runs', true], ['targets', false]]);
  });

  it('keeps a property id as guest text, apart from the labels the page writes itself', () => {
    const [group] = groupHits({ ...EMPTY, properties: [{ run_id: RUN, guest: { note: 'n', id: '<img src=x onerror=alert(1)>' } }] });
    const [row] = group.rows;
    expect(row.guestText).toBe('<img src=x onerror=alert(1)>');
    expect(row.main).toBeUndefined();
    expect(JSON.stringify({ ...row, guestText: undefined })).not.toContain('<img');
  });

  it('drops a hit whose address cannot be built, instead of linking to a made-up path', () => {
    const groups = groupHits({ ...EMPTY, runs: [{ id: '../x', target: 't', state: 'failed' }, { id: RUN, target: 't', state: 'failed' }] });
    expect(groups[0].rows.map((r) => r.path)).toEqual([`/runs/${RUN}`]);
    expect(groups[0].rows[0].index).toBe(0);
  });

  it('encodes a target name in the address it filters by', () => {
    const [group] = groupHits({ ...EMPTY, targets: [{ name: 'a&b=c d' }] });
    expect(group.rows[0].path).toBe('/runs?target=a%26b%3Dc%20d');
  });

  it('says in plain words what a seed hit is: the seed a run started with, or a trial that ran it', () => {
    const [group] = groupHits({ ...EMPTY, seeds: [{ run_id: RUN, seed: 7, trial: 2 }, { run_id: RUN, seed: 7, trial: null }] });
    expect(group.rows.map((r) => r.detail)).toEqual([`Trial 2 of ${RUN}`, `Run ${RUN} started with it`]);
  });
});

describe('scopeNote', () => {
  it('says how far a search looked, so "no matches" is not read as "never stored"', () => {
    const note = scopeNote(EMPTY.scope);
    expect(note).toContain('1000 newest runs');
    expect(note).toContain('100 newest runs');
    expect(note).toMatch(/whole run or failure id/);
  });
});
