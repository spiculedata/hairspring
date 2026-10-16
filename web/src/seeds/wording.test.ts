import { describe, expect, it } from 'vitest';
import { trialRow } from '../trial/fixtures';
import type { TrialClass } from '../trial/types';
import { countCodes } from './strip';
import {
  classLabel,
  classMark,
  keptText,
  stoppedRunNote,
  stripBinText,
  stripSummary,
  verdictCellText,
} from '../wording';

const CLASSES: TrialClass[] = ['pass', 'violation', 'inconclusive', 'unusable', 'infra', 'excluded'];

describe('class labels and marks', () => {
  it('give every class its own words and its own mark, so colour is never the only signal', () => {
    expect(new Set(CLASSES.map(classLabel)).size).toBe(CLASSES.length);
    expect(new Set(CLASSES.map(classMark)).size).toBe(CLASSES.length);
    for (const c of CLASSES) expect(classMark(c).length).toBeGreaterThan(0);
  });

  it('never calls anything but a rated clean pass a pass', () => {
    for (const c of CLASSES.filter((c) => c !== 'pass')) expect(classLabel(c)).not.toMatch(/\bpass\b/i);
  });
});

describe('verdictCellText', () => {
  it('shows a degraded pass as excluded and says why, never as a pass', () => {
    const text = verdictCellText(trialRow({ verdict: 'pass', class: 'excluded', degraded: true, rated: false, failing_count: 0 }));
    expect(text).toContain('excluded');
    expect(text).toContain('degraded');
    expect(text).not.toMatch(/\bpass\b/i);
  });

  it('says why an unreported or unrated pass was excluded', () => {
    expect(verdictCellText(trialRow({ verdict: 'pass', class: 'excluded', report_missing: true }))).toContain('no report');
    expect(verdictCellText(trialRow({ verdict: 'pass', class: 'excluded', rated: false }))).toContain('not rated');
  });

  it('keeps a perturbing trial visible as perturbing', () => {
    expect(verdictCellText(trialRow({ verdict: 'pass', class: 'pass', perturbing: true }))).toContain('perturbing');
  });

  it('shows an inconclusive trial as inconclusive', () => {
    const text = verdictCellText(trialRow({ verdict: 'inconclusive', class: 'inconclusive', timed_out: true }));
    expect(text).toContain('inconclusive');
    expect(text).not.toMatch(/\bpass\b/i);
  });
});

describe('keptText', () => {
  it('says plainly whether detail was kept, and when a cap dropped it', () => {
    expect(keptText('kept')).toBe('yes');
    expect(keptText('none')).toBe('no');
    expect(keptText('dropped_cap')).toBe('dropped (size cap)');
    expect(keptText('something-new')).toBe('unknown');
  });
});

describe('strip text', () => {
  it('describes one trial and a group of trials with their counts, worst first', () => {
    expect(stripBinText(15, 16, countCodes('v'))).toBe('Trial 15: 1 violation');
    expect(stripBinText(1200, 1300, countCodes('p'.repeat(98) + 'ee'))).toBe('Trials 1,200 to 1,299: 2 excluded, 98 pass');
    expect(stripBinText(0, 4, countCodes('pvip'))).toBe('Trials 0 to 3: 1 violation, 1 inconclusive, 2 pass');
  });

  it('summarises a whole run by class, naming only the classes that occur', () => {
    expect(stripSummary(countCodes('p'.repeat(90) + 'vvv' + 'i'.repeat(4) + 'e'.repeat(3)))).toBe(
      '100 judged: 3 violation, 4 inconclusive, 3 excluded, 90 pass',
    );
    expect(stripSummary(countCodes(''))).toBe('0 judged');
  });
});

describe('stoppedRunNote', () => {
  it('says the run stopped at its first failure and that later trials were not judged', () => {
    const text = stoppedRunNote(16);
    expect(text).toMatch(/stopped at its first failure/i);
    expect(text).toContain('after trial 16');
    expect(text).toMatch(/not judged/);
    expect(text).toMatch(/not a rate/i);
  });

  it('does not need a trial number', () => {
    expect(stoppedRunNote(undefined)).toMatch(/stopped at its first failure/i);
  });
});
