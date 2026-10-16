import { describe, expect, it } from 'vitest';
import { detail, HEALTH_CLEAN, property, proc, record, witness } from './fixtures';
import { failingProperties, healthCounters, lanePosition, laneRows, ordinalOf, processState, witnessThreadEvals } from './model';

describe('failingProperties', () => {
  it('lists an always property that was false, with its first-false witness and the false count', () => {
    const [f] = failingProperties([property()]);
    expect(f.reason).toBe('always_false');
    expect(f.count).toBe(1);
    expect(f.witness?.ord).toBe(2301);
  });

  it('lists an unreachable property that was hit, using its first-true witness', () => {
    const w = witness({ thread: 'late', ord: 7 });
    const [f] = failingProperties([property({ kind: 'unreachable', hits: 4, trues: 4, falses: 0, first_false: null, first_true: w })]);
    expect(f.reason).toBe('unreachable_hit');
    expect(f.count).toBe(4);
    expect(f.witness).toBe(w);
  });

  it('does not list sometimes, reachable or passing properties', () => {
    const rows = [
      property({ kind: 'sometimes', falses: 9 }),
      property({ kind: 'reachable', falses: 9 }),
      property({ falses: 0, first_false: null }),
      property({ kind: 'unreachable', hits: 0 }),
    ];
    expect(failingProperties(rows)).toEqual([]);
  });

  it('keeps a failure whose witness is missing, rather than dropping it', () => {
    const [f] = failingProperties([property({ first_false: null })]);
    expect(f.witness).toBeNull();
  });

  it('does not change its input', () => {
    const rows = [property()];
    const before = JSON.stringify(rows);
    failingProperties(rows);
    expect(JSON.stringify(rows)).toBe(before);
  });
});

describe('lanePosition', () => {
  it('places a witness by its own thread count, clamped to the lane', () => {
    expect(lanePosition(2301, 2410)).toBeCloseTo(0.9548, 3);
    expect(lanePosition(5000, 2410)).toBe(1);
    expect(lanePosition(0, 2410)).toBe(0);
    expect(lanePosition(3, 0)).toBe(0);
  });
});

describe('witnessThreadEvals', () => {
  it('finds the thread count for the witness process and thread', () => {
    expect(witnessThreadEvals(record().props.procs, witness())).toBe(2410);
  });
  it('is undefined for a thread the process never reported, and for an unstable key reused in another process', () => {
    expect(witnessThreadEvals(record().props.procs, witness({ thread: 'ghost' }))).toBeUndefined();
    expect(witnessThreadEvals(record().props.procs, witness({ proc: 'other' }))).toBeUndefined();
  });
});

describe('ordinalOf', () => {
  it('writes an ordinal with the usual suffix and grouping', () => {
    expect([1, 2, 3, 4, 11, 12, 13, 21, 22, 101, 2301].map(ordinalOf)).toEqual(['1st', '2nd', '3rd', '4th', '11th', '12th', '13th', '21st', '22nd', '101st', '2,301st']);
  });
});

describe('laneRows', () => {
  const failing = failingProperties([property()]);

  it('has one row per thread of every process, in report order, with witness marks on the right lane', () => {
    const rows = laneRows(record().props.procs, failing);
    expect(rows.map((r) => `${r.proc}/${r.thread}`)).toEqual(['main/writer#1', 'main/ckpt#1', 'main/t#3']);
    expect(rows[0].marks).toEqual([{ ord: 2301, position: lanePosition(2301, 2410), propertyIds: ['wal.no-lost-commit'] }]);
    expect(rows[1].marks).toEqual([]);
    expect(rows[2].keyStable).toBe(false);
  });

  it('puts two failing properties that share a witness on one mark', () => {
    const two = failingProperties([property(), property({ id: 'wal.other' })]);
    const rows = laneRows(record().props.procs, two);
    expect(rows[0].marks).toHaveLength(1);
    expect(rows[0].marks[0].propertyIds).toEqual(['wal.no-lost-commit', 'wal.other']);
  });

  it('keeps one row per process and thread even when two processes use the same thread key', () => {
    const procs = [proc({ tag: 'a' }), proc({ tag: 'b' })];
    expect(laneRows(procs, [])).toHaveLength(6);
  });
});

describe('processState', () => {
  it('says ok for an exit with status 0', () => {
    expect(processState(proc())).toEqual({ kind: 'ok', text: 'ended: exit, status 0' });
  });
  it('says a nonzero exit and a signal plainly', () => {
    expect(processState(proc({ end: { reason: 'Exit', status: 3, sites: 1, evals: 1, dropped: 0, threads: 0 } })).text).toBe('ended: exit, status 3');
    expect(processState(proc({ end: { reason: 'Signal', status: 11, sites: 1, evals: 1, dropped: 0, threads: 0 } })).text).toBe('ended: signal, status 11');
  });
  it('flags a process that never sent an end report', () => {
    expect(processState(proc({ end: null }))).toEqual({ kind: 'missing', text: 'no end report' });
  });
});

describe('healthCounters', () => {
  it('lists nothing but the accepted count for a clean trial', () => {
    expect(healthCounters(HEALTH_CLEAN).problems).toEqual([]);
    expect(healthCounters(HEALTH_CLEAN).accepted).toBe(211);
  });
  it('lists every nonzero loss counter with a label, and ignores unknown names', () => {
    const c = healthCounters({ ...HEALTH_CLEAN, malformed: 2, over_limit: 5, mystery: 9 });
    expect(c.problems).toEqual([
      { key: 'malformed', label: 'malformed event lines', count: 2 },
      { key: 'over_limit', label: 'lines dropped by a limit', count: 5 },
    ]);
  });
  it('treats a missing or negative counter as zero', () => {
    expect(healthCounters({}).accepted).toBe(0);
    expect(healthCounters({ malformed: -3 }).problems).toEqual([]);
  });
});

describe('detail fixture', () => {
  it('is a failing kept trial', () => {
    expect(detail().trial.verdict).toBe('violation');
  });
});
