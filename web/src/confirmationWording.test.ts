import { describe, expect, it } from 'vitest';
import {
  CHECK_KIND_LABELS,
  CHECK_STATUS_LABELS,
  ENVIRONMENT_DIFFERS_NOTE,
  FINDING_LABELS,
  FINDING_MEANINGS,
  LADDER_NOTE,
  LADDER_RUNGS,
  NO_CHECKS_HINT,
  NO_CHECKS_TEXT,
  countsText,
  noChecksCommand,
} from './confirmationWording';

describe('finding wording (the server derives these six states)', () => {
  it('has a label and a documented meaning for each, and no more', () => {
    const states = ['candidate', 'rerun_repeated', 'confirmed', 'verified', 'not_reproducible', 'control_failed'].sort();
    expect(Object.keys(FINDING_LABELS).sort()).toEqual(states);
    expect(Object.keys(FINDING_MEANINGS).sort()).toEqual(states);
  });
  it('keeps the ladder in the server order, rungs only', () => {
    expect(LADDER_RUNGS).toEqual(['candidate', 'rerun_repeated', 'confirmed', 'verified']);
  });
  it('says a rerun alone has no replay yet, as the server documents', () => {
    expect(FINDING_MEANINGS.rerun_repeated).toContain('no replay of the bundle has yet');
  });
  it('never says fixed or resolved except to deny it', () => {
    for (const text of Object.values(FINDING_MEANINGS)) expect(text).not.toMatch(/\b(resolved|solved)\b|\bis fixed\b|\bwas fixed\b/i);
    expect(LADDER_NOTE).toContain('None of the states says the failure is fixed.');
  });
  it('never uses the word "reproduced" or "found"', () => {
    for (const text of [...Object.values(FINDING_MEANINGS), ...Object.values(FINDING_LABELS), LADDER_NOTE]) {
      expect(text).not.toMatch(/\breproduced\b|\bfound\b/i);
    }
  });
});

describe('check wording', () => {
  it('labels the four kinds and four statuses the server stores', () => {
    expect(CHECK_KIND_LABELS).toEqual({
      same_seed_rerun: 'Same-seed rerun',
      bundle_replay: 'Bundle replay',
      fixed_control: 'Fixed control',
      strict_verify: 'Strict verify',
    });
    expect(CHECK_STATUS_LABELS).toEqual({ pass: 'Pass', fail: 'Fail', inconclusive: 'Inconclusive', skipped: 'Skipped' });
  });
  it('writes counts as given, with grouped thousands', () => {
    expect(countsText({ passed: 3, failed: 0, inconclusive: 0 })).toBe('3 passed, 0 failed, 0 inconclusive');
    expect(countsText({ passed: 1200, failed: 1, inconclusive: 2 })).toBe('1,200 passed, 1 failed, 2 inconclusive');
  });
  it('says an environment difference does not show the bug is fixed', () => {
    expect(ENVIRONMENT_DIFFERS_NOTE).toBe('inconclusive: environment differs from the recording; this does not show the bug is fixed');
  });
  it('gives the real command for a failure with no checks', () => {
    expect(NO_CHECKS_TEXT).toBe('No checks recorded. This failure has not been confirmed.');
    expect(noChecksCommand('fail_0123456789abcdef')).toBe('hairspring confirm fail_0123456789abcdef');
    for (const bad of ['fail_0123', '<script>', 'fail_0123456789ABCDEF', 'x fail_0123456789abcdef', 'fail_0123456789abcdef\n']) {
      expect(noChecksCommand(bad)).toBe('hairspring confirm <failure id>');
    }
    expect(NO_CHECKS_HINT).toContain('Hermit');
    expect(NO_CHECKS_HINT).toContain('Linux on x86-64');
  });
});
