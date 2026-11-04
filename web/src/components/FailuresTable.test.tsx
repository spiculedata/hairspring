import { renderToString } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { absoluteTime } from '../format';
import { readSource } from '../readSource';
import type { FailureSummary } from '../types';
import { FailuresTable, newestFirst } from './FailuresTable';

const failure = (over: Partial<FailureSummary> = {}): FailureSummary => ({
  id: 'f_0123',
  run_id: 'run_0123456789abcdef',
  target: 'pubsub-atomic',
  message: 'assertion failed: balance never negative',
  original_len: 412,
  shrunk_len: 9,
  bundle_hash: 'ab'.repeat(32),
  created_at: Date.UTC(2026, 9, 30, 15, 44, 50),
  ...over,
});
const text = (html: string): string => html.replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' ');

describe('newestFirst', () => {
  it('orders by creation time, newest first, without changing the list it was given', () => {
    const list = [failure({ id: 'a', created_at: 1 }), failure({ id: 'c', created_at: 3 }), failure({ id: 'b', created_at: 2 })];
    const copy = [...list];
    expect(newestFirst(list).map((f) => f.id)).toEqual(['c', 'b', 'a']);
    expect(list).toEqual(copy);
  });
  it('keeps the server order for failures with the same time', () => {
    const list = [failure({ id: 'x', created_at: 5 }), failure({ id: 'y', created_at: 5 })];
    expect(newestFirst(list).map((f) => f.id)).toEqual(['x', 'y']);
  });
});

describe('FailuresTable', () => {
  it('shows each failure with its target, message, step counts, an absolute time and links to its run and its bundle page', () => {
    const html = renderToString(<FailuresTable failures={[failure()]} />);
    expect(html).toContain('<a href="/failures/f_0123"');
    expect(html).toContain('<a href="/runs/run_0123456789abcdef"');
    expect(text(html)).toContain('pubsub-atomic');
    expect(text(html)).toContain('assertion failed: balance never negative');
    expect(text(html)).toContain('412');
    expect(text(html)).toContain(absoluteTime(Date.UTC(2026, 9, 30, 15, 44, 50)));
    expect(html).toContain('dateTime="2026-10-30T15:44:50.000Z"');
  });

  it('lists the newest first', () => {
    const html = renderToString(<FailuresTable failures={[failure({ id: 'f_old', created_at: 1 }), failure({ id: 'f_new', created_at: 2 })]} />);
    expect(html.indexOf('f_new')).toBeLessThan(html.indexOf('f_old'));
  });

  it('shows the message as escaped guest text, never as markup', () => {
    const html = renderToString(<FailuresTable failures={[failure({ message: '<img src=x onerror=1>‮' })]} />);
    expect(html).not.toContain('<img');
    expect(html).not.toContain('‮');
    expect(html).toContain('guest-text');
  });

  it('does not link an id that is not plain, so nothing else can reach a path', () => {
    const html = renderToString(<FailuresTable failures={[failure({ id: '../x?y', run_id: 'a b' })]} />);
    expect(html).not.toContain('href="../');
    expect(html).not.toContain('href="/runs/a');
    expect(text(html)).toContain('../x?y');
  });

  it('says nothing the server did not: no status, no "confirmed", "fixed", "resolved" or "verified"', () => {
    const html = renderToString(<FailuresTable failures={[failure(), failure({ id: 'f_2' })]} />);
    expect(html).not.toMatch(/confirm|fixed|resolved|verified|badge/i);
  });

  it('adds a State column, in the confirmation words, only when the server sent a finding (a list filtered by state)', () => {
    const withFinding = renderToString(
      <FailuresTable failures={[failure({ finding: { status: 'confirmed', checks: 2, environment_pinned: false } }), failure({ id: 'f_2', finding: { status: 'candidate', checks: 0, environment_pinned: false } })]} />,
    );
    expect(text(withFinding)).toContain('State');
    expect(text(withFinding)).toContain('Confirmed');
    expect(text(withFinding)).toContain('Candidate');
    expect(withFinding).toContain('Server state: confirmed');
    const without = renderToString(<FailuresTable failures={[failure()]} />);
    expect(text(without)).not.toContain('State');
    expect(without.match(/<th[ >]/g)).toHaveLength(7);
    expect(withFinding.match(/<th[ >]/g)).toHaveLength(8);
  });

  it('shows a state this browser does not know as Unknown, not as a rung', () => {
    const html = renderToString(<FailuresTable failures={[failure({ finding: { status: 'brand_new' as never, checks: 1, environment_pinned: false } })]} />);
    expect(text(html)).toContain('Unknown');
  });

  it('marks the step columns optional so a phone can drop them, and uses the runs table rules for reflow', () => {
    const html = renderToString(<FailuresTable failures={[failure()]} />);
    expect(html).toContain('class="runs failures"');
    expect(html.match(/class="num opt"/g)).toHaveLength(4);
    expect(readSource('styles.css')).toMatch(/table\.failures[^{]*\{[^}]*overflow-wrap:\s*anywhere/);
  });
});
