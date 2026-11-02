import { describe, expect, it } from 'vitest';
import {
  PROPERTY_STATUSES,
  REPLAY_STATUSES,
  RUN_STATES,
  UNKNOWN_LABEL,
  propertyStatusEntry,
  replayEntry,
  runStateEntry,
  type Tone,
} from './statusMap';
import { statusLabel } from './wording';

const TONES: readonly Tone[] = ['good', 'bad', 'busy', 'idle', 'gap'];

describe('run states (the server sends one of these words for a run)', () => {
  it.each([
    ['queued', 'Queued', 'idle'],
    ['running', 'Running', 'busy'],
    ['failed', 'Failed', 'bad'],
    ['error', 'Error', 'bad'],
    ['cancelled', 'Cancelled', 'idle'],
  ] as const)('maps %s to "%s" in the %s tone', (state, label, tone) => {
    expect(runStateEntry(state)).toEqual({ label, tone });
  });

  it('has no entry for "passed": that word needs the run outcome rules, which know the trial count and completion', () => {
    expect(Object.keys(RUN_STATES)).not.toContain('passed');
    expect(runStateEntry('passed')).toEqual({ label: UNKNOWN_LABEL, tone: 'idle' });
  });

  it('shows an unknown state as a neutral "Unknown", never in a pass tone', () => {
    for (const state of ['', 'PASSED', 'ok', 'success', 'toString', '__proto__', 'constructor']) {
      expect(runStateEntry(state)).toEqual({ label: 'Unknown', tone: 'idle' });
    }
  });
});

describe('property statuses', () => {
  it.each([
    ['violation', 'Failed', 'bad'],
    ['unmet', 'Never reached', 'gap'],
    ['unrated', 'Unrated', 'idle'],
    ['pass', 'Held', 'good'],
  ] as const)('maps %s to "%s" in the %s tone', (status, label, tone) => {
    expect(propertyStatusEntry(status)).toEqual({ label, tone });
  });

  it('takes every label from the wording module, so the filter and the table cannot disagree', () => {
    for (const status of Object.keys(PROPERTY_STATUSES) as Array<keyof typeof PROPERTY_STATUSES>) {
      expect(propertyStatusEntry(status).label).toBe(statusLabel(status));
    }
  });

  it('shows an unknown status as a neutral "Unknown"', () => {
    expect(propertyStatusEntry('maybe')).toEqual({ label: 'Unknown', tone: 'idle' });
    expect(propertyStatusEntry('toString')).toEqual({ label: 'Unknown', tone: 'idle' });
  });

  it('never gives UNMET the failure tone, and never gives a pass tone to anything but Held', () => {
    expect(propertyStatusEntry('unmet').tone).not.toBe('bad');
    for (const [status, entry] of Object.entries(PROPERTY_STATUSES)) {
      if (entry.tone === 'good') expect(status).toBe('pass');
    }
  });
});

describe('replay results', () => {
  it('maps a failing replay to the failure tone and a clean one to a neutral tone, since one replay proves nothing', () => {
    expect(replayEntry(true)).toEqual({ label: 'Failed again on replay', tone: 'bad' });
    expect(replayEntry(false)).toEqual({ label: 'Did not fail on replay', tone: 'idle' });
    expect(REPLAY_STATUSES.didNotFail.tone).not.toBe('good');
  });
});

describe('the whole table', () => {
  const all = [...Object.values(RUN_STATES), ...Object.values(PROPERTY_STATUSES), ...Object.values(REPLAY_STATUSES)];
  it('gives every entry words and a known tone', () => {
    for (const e of all) {
      expect(e.label.trim()).not.toBe('');
      expect(TONES).toContain(e.tone);
    }
  });
  it('uses the pass tone only for a word that is about a held property or a run with no failure seen', () => {
    expect(all.filter((e) => e.tone === 'good').map((e) => e.label)).toEqual(['Held']);
  });
  it('never writes the words found, discovered, unassisted, flaky or reproduced', () => {
    for (const e of all) expect(e.label).not.toMatch(/\bfound\b|discovered|unassisted|flaky|reproduc/i);
  });
});
