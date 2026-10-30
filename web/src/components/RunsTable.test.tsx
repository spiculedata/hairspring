import { renderToString } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import type { Run } from '../types';
import { readSource } from '../readSource';
import { RunsTable } from './RunsTable';

const run = (over: Partial<Run> = {}): Run => ({
  id: 'run_0123456789abcdef',
  target: 'pubsub-atomic',
  backend: 'sim',
  state: 'passed',
  seed: 1,
  created_at: 1_000,
  started_at: 1_000,
  finished_at: 2_000,
  steps: 400_000,
  trials: 2,
  error: null,
  ...over,
});

describe('RunsTable', () => {
  it('marks the Steps column optional in its header and every row, so a phone can drop it', () => {
    const html = renderToString(<RunsTable runs={[run(), run({ id: 'run_b' })]} />);
    expect(html.match(/class="num opt"/g)).toHaveLength(3);
    expect(html).toContain('class="runs"');
  });

  it('keeps the step count reachable on a phone, as a small note under the id', () => {
    const html = renderToString(<RunsTable runs={[run({ steps: 5111 })]} />);
    expect(html).toMatch(/<span class="muted steps-note">5,111 steps<\/span>/);
  });

  it('says what an empty list means and what to do', () => {
    const html = renderToString(<RunsTable runs={[]} />);
    expect(html).toContain('No runs yet');
    expect(html).toContain('hairspring test');
  });
});

describe('the Runs table on a phone, from the stylesheet', () => {
  const css = readSource('styles.css');
  const phone = css.slice(css.indexOf('@media (max-width: 40rem)'));

  it('hides the optional column and lets long ids and badges wrap, so the page never scrolls sideways', () => {
    expect(phone).toMatch(/table\.runs \.opt\s*\{\s*display:\s*none/);
    expect(phone).toMatch(/table\.runs td:first-child\s*\{[^}]*overflow-wrap:\s*anywhere/);
  });

  it('never breaks a word inside a badge or any other cell but the id: only the id column may break anywhere', () => {
    expect(phone).not.toMatch(/table\.runs td\s*\{[^}]*overflow-wrap:\s*anywhere/);
    expect(css).toMatch(/\.badge \{[^}]*white-space:\s*nowrap/);
  });

  it('shows the relative time only on a phone, so a wide absolute time does not squeeze the State column', () => {
    expect(phone).toMatch(/table\.runs time\s*\{\s*display:\s*none/);
  });

  it('shows the step note only on a phone', () => {
    expect(css).toMatch(/\.steps-note\s*\{\s*display:\s*none/);
    expect(phone).toMatch(/\.steps-note\s*\{[^}]*display:\s*block/);
  });
});
