import { describe, expect, it } from 'vitest';
import type { Worker } from '../types';
import { capabilityLines, hiddenIdNote, idColumnShown, jobsSummary, MAX_CAPABILITY_LINES, MAX_CAPABILITY_TEXT } from './workersModel';
import { CAPS_NOT_STATED, CAPS_UNREADABLE } from './workersWording';
import type { MeState } from '../me';

const worker = (extra: Partial<Worker> = {}): Worker => ({ name: 'w', last_seen: 1, registered_at: 1, ...extra });
const ready = (role: 'read' | 'run' | 'admin'): MeState => ({
  status: 'ready',
  me: { role, open_mode: false, can: { read: true, run: role !== 'read', admin: role === 'admin' }, roles: ['read', 'run', 'admin'] },
});

describe('capabilityLines', () => {
  it('lists each key with its values, arrays joined', () => {
    expect(capabilityLines(worker({ capabilities: { backends: ['sim', 'embedded'], max_trials: 100 } }))).toEqual([
      'backends: sim, embedded',
      'max_trials: 100',
    ]);
  });

  it('says nothing was stated for an empty object, and that the text could not be read when it was not JSON', () => {
    expect(capabilityLines(worker({ capabilities: {}, caps_json: '{}' }))).toEqual([CAPS_NOT_STATED]);
    expect(capabilityLines(worker({ capabilities: null, caps_json: 'not json' }))).toEqual([CAPS_UNREADABLE]);
  });

  it('does not call the JSON text null unreadable', () => {
    expect(capabilityLines(worker({ capabilities: null, caps_json: 'null' }))).toEqual([CAPS_NOT_STATED]);
  });

  it('treats a server that sends no capabilities field as nothing stated', () => {
    expect(capabilityLines(worker())).toEqual([CAPS_NOT_STATED]);
  });

  it('shows a value that is not an object as its JSON text', () => {
    expect(capabilityLines(worker({ capabilities: ['a', 1] }))).toEqual(['["a",1]']);
  });

  it('shows nested values as JSON and cuts long text and long lists', () => {
    const long = 'x'.repeat(MAX_CAPABILITY_TEXT + 50);
    const lines = capabilityLines(worker({ capabilities: { nested: { a: 1 }, long } }));
    expect(lines[0]).toBe('nested: {"a":1}');
    expect(lines[1].length).toBeLessThanOrEqual(MAX_CAPABILITY_TEXT + 1);
    expect(lines[1].endsWith('…')).toBe(true);
    const many = Object.fromEntries(Array.from({ length: MAX_CAPABILITY_LINES + 4 }, (_, i) => [`k${i}`, i]));
    const cut = capabilityLines(worker({ capabilities: many }));
    expect(cut).toHaveLength(MAX_CAPABILITY_LINES + 1);
    expect(cut[MAX_CAPABILITY_LINES]).toBe('and 4 more');
  });
});

describe('jobsSummary', () => {
  it('names nothing for a worker that holds nothing', () => {
    expect(jobsSummary(worker({ leased_jobs: 0, current_jobs: [] }))).toEqual({ jobs: [], more: 0 });
    expect(jobsSummary(worker())).toEqual({ jobs: [], more: 0 });
  });

  it('never reports a negative number when the count is lower than the jobs named', () => {
    const jobs = [{ run_id: 'r1', kind: 'run', lease_until: 5, attempts: 1 }];
    expect(jobsSummary(worker({ leased_jobs: 0, current_jobs: jobs })).more).toBe(0);
  });

  it('reports the jobs held that are not named', () => {
    const jobs = [{ run_id: 'r1', kind: 'run', lease_until: 5, attempts: 1 }];
    expect(jobsSummary(worker({ leased_jobs: 4, current_jobs: jobs }))).toEqual({ jobs, more: 3 });
  });
});

describe('the id column', () => {
  it('is shown when any worker has an id, and not otherwise', () => {
    expect(idColumnShown([worker({ id: 'wrk-1' })])).toBe(true);
    expect(idColumnShown([worker()])).toBe(false);
    expect(idColumnShown([])).toBe(false);
  });

  it('is explained for a read caller only', () => {
    expect(hiddenIdNote(ready('read'))).toBe(true);
    expect(hiddenIdNote(ready('run'))).toBe(false);
    expect(hiddenIdNote(ready('admin'))).toBe(false);
    expect(hiddenIdNote({ status: 'loading' })).toBe(false);
    expect(hiddenIdNote({ status: 'unavailable' })).toBe(false);
  });
});
