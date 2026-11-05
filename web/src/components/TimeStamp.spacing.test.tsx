import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { AttentionFailures, AttentionRuns } from '../overview/OverviewSections';
import type { FailureSummary, Run } from '../types';
import { TimeStamp } from './TimeStamp';

const THEN = Date.UTC(2026, 10, 4, 17, 54, 47);
const text = (html: string): string => html.replace(/<[^>]*>/g, '');

describe('text order reads as separate words', () => {
  it('puts a space between the absolute and the relative time', () => {
    const t = text(renderToStaticMarkup(<TimeStamp ms={THEN} now={THEN + 90_000} timeZone="UTC" />));
    expect(t).toContain('17:54:47 UTC 1m ago');
  });

  it('keeps the target name apart from the time in the overview lists', () => {
    const run = { id: 'run_1', target: 'lock-lease', state: 'failed', trials: 0, created_at: THEN, error: null } as unknown as Run;
    const failure = { id: 'f1', run_id: 'run_1', target: 'pubsub-bug', message: 'm', created_at: THEN } as unknown as FailureSummary;
    const runs = text(renderToStaticMarkup(<AttentionRuns runs={[run]} />));
    const failures = text(renderToStaticMarkup(<AttentionFailures failures={[failure]} limit={5} />));
    expect(runs).toMatch(/lock-lease\s+\d/);
    expect(failures).toMatch(/pubsub-bug\s+\d/);
  });
});
