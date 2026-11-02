/// <reference types="node" />
import { readdirSync } from 'node:fs';
import { sep } from 'node:path';
import { renderToString } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { contrastRatio } from '../contrast';
import { readSource } from '../readSource';
import type { PropertyStatus } from '../types';
import { StatusBadge } from './StatusBadge';

const text = (html: string): string => html.replace(/<[^>]+>/g, '');
const complete = { kind: 'complete' } as const;

describe('StatusBadge for a run', () => {
  it('shows a passed run as "No failure seen" with its scope and the server word in the tooltip', () => {
    const html = renderToString(<StatusBadge kind="run" state="passed" trials={30} completion={complete} />);
    expect(html).toContain('No failure seen');
    expect(html).toContain('30 trials');
    expect(html).toContain('title="Server state: passed"');
    expect(html).not.toMatch(/>passed</);
  });
  it('shows "Stopped early" with trials run of planned', () => {
    const html = renderToString(
      <StatusBadge kind="run" state="passed" trials={2} completion={{ kind: 'time_or_step_limited', trials_run: 2, trials_budget: 30 }} />,
    );
    expect(html).toContain('Stopped early');
    expect(html).toContain('2 of 30 planned trials');
  });
  it('is neutral, with no pass tone, while the completion is not known', () => {
    const html = renderToString(<StatusBadge kind="run" state="passed" trials={30} />);
    expect(html).toContain('badge-idle');
    expect(html).not.toContain('badge-good');
    expect(html).not.toContain('No failure seen');
    expect(text(html)).toContain('Finished');
  });
  it('never reads as a pass when no trial ran', () => {
    for (const completion of [undefined, complete]) {
      const html = renderToString(<StatusBadge kind="run" state="passed" trials={0} completion={completion} />);
      expect(text(html)).toBe('No trials run');
      expect(html).not.toContain('badge-good');
    }
  });
  it('has a space between the label and its scope, so it reads right without the stylesheet', () => {
    expect(text(renderToString(<StatusBadge kind="run" state="passed" trials={30} completion={complete} />))).toBe('No failure seen 30 trials');
  });
  it('puts the scope in its own small element, so it does not take the heading size', () => {
    expect(renderToString(<StatusBadge kind="run" state="passed" trials={30} completion={complete} />)).toContain('badge-scope');
  });
  it('gives no scope when the run list has no completion to scope it by trial count alone', () => {
    const html = renderToString(<StatusBadge kind="run" state="failed" trials={3} />);
    expect(html).toContain('>Failed<');
    expect(html).not.toContain('trials');
  });
  it.each([
    ['queued', 'Queued', 'badge-idle'],
    ['running', 'Running', 'badge-busy'],
    ['failed', 'Failed', 'badge-bad'],
    ['error', 'Error', 'badge-bad'],
    ['cancelled', 'Cancelled', 'badge-idle'],
  ])('shows the server state %s as "%s"', (state, label, cls) => {
    const html = renderToString(<StatusBadge kind="run" state={state} trials={1} />);
    expect(text(html)).toBe(label);
    expect(html).toContain(cls);
    expect(html).toContain(`title="Server state: ${state}"`);
  });
  it('shows an unknown state as a neutral "Unknown" and keeps the server word in the tooltip', () => {
    const html = renderToString(<StatusBadge kind="run" state="succeeded" trials={9} completion={complete} />);
    expect(text(html)).toBe('Unknown');
    expect(html).toContain('badge-idle');
    expect(html).not.toContain('badge-good');
    expect(html).toContain('title="Server state: succeeded"');
  });
  it('escapes a state word in the tooltip', () => {
    const html = renderToString(<StatusBadge kind="run" state={'"><img src=x onerror=1>'} trials={1} />);
    expect(html).not.toContain('<img');
  });
});

describe('StatusBadge for a property', () => {
  it.each([
    ['violation', 'Failed', 'badge-bad'],
    ['unmet', 'Never reached', 'badge-gap'],
    ['unrated', 'Unrated', 'badge-idle'],
    ['pass', 'Held', 'badge-good'],
  ] satisfies ReadonlyArray<readonly [PropertyStatus, string, string]>)('shows %s as "%s"', (status, label, cls) => {
    const html = renderToString(<StatusBadge kind="property" status={status} />);
    expect(text(html)).toBe(label);
    expect(html).toContain(cls);
    expect(html).toContain(`title="Server status: ${status}"`);
  });
  it('shows UNMET as a coverage gap, never in the failure tone', () => {
    expect(renderToString(<StatusBadge kind="property" status="unmet" />)).not.toContain('badge-bad');
  });
  it('shows an unknown status as a neutral "Unknown"', () => {
    const html = renderToString(<StatusBadge kind="property" status="maybe" />);
    expect(text(html)).toBe('Unknown');
    expect(html).toContain('badge-idle');
  });
});

describe('StatusBadge for a replay', () => {
  it('says "Failed again on replay" in the failure tone', () => {
    const html = renderToString(<StatusBadge kind="replay" failedAgain />);
    expect(text(html)).toBe('Failed again on replay');
    expect(html).toContain('badge-bad');
  });
  it('says "Did not fail on replay" in a neutral tone, since one replay is not a clean bill of health', () => {
    const html = renderToString(<StatusBadge kind="replay" failedAgain={false} />);
    expect(text(html)).toBe('Did not fail on replay');
    expect(html).toContain('badge-idle');
    expect(html).not.toContain('badge-good');
  });
});

describe('StatusBadge accessibility', () => {
  const cases = [
    <StatusBadge key="a" kind="run" state="passed" trials={3} completion={complete} />,
    <StatusBadge key="b" kind="run" state="failed" trials={3} />,
    <StatusBadge key="c" kind="run" state="odd" trials={3} />,
    <StatusBadge key="d" kind="property" status="unmet" />,
    <StatusBadge key="e" kind="property" status="pass" />,
    <StatusBadge key="f" kind="replay" failedAgain />,
  ];
  it('always carries its meaning in text, so colour is never the only signal', () => {
    for (const c of cases) {
      const html = renderToString(c);
      expect(text(html).trim().length, html).toBeGreaterThan(3);
      expect(html).not.toMatch(/aria-hidden="true"[^>]*>[A-Za-z]/);
    }
  });
  it('draws every tone from the status token pairs, and each pair passes 4.5:1 in both themes', () => {
    const css = readSource('styles.css');
    const tones = { good: 'pass', bad: 'fail', busy: 'live', idle: 'neutral', gap: 'gap' } as const;
    const light = css.slice(0, css.indexOf('@media (prefers-color-scheme: dark)'));
    const dark = css.slice(css.indexOf('@media (prefers-color-scheme: dark)'));
    for (const [tone, token] of Object.entries(tones)) {
      expect(css, tone).toMatch(new RegExp(`\\.badge-${tone} \\{[^}]*color: var\\(--${token}\\); background: var\\(--${token}-bg\\)`));
      for (const [name, block] of [['light', light], ['dark', dark]] as const) {
        const fg = new RegExp(`--${token}: (#[0-9a-f]{6})`, 'i').exec(block)?.[1];
        const bg = new RegExp(`--${token}-bg: (#[0-9a-f]{6})`, 'i').exec(block)?.[1];
        expect(fg && bg, `${token} ${name}`).toBeTruthy();
        expect(contrastRatio(fg!, bg!), `${token} ${name}`).toBeGreaterThanOrEqual(4.5);
      }
    }
  });
  it('gives each tone its own mark as generated content with no spoken text', () => {
    const css = readSource('styles.css');
    const marks = new Set<string>();
    for (const tone of ['good', 'bad', 'busy', 'idle', 'gap']) {
      const m = new RegExp(`\\.badge-${tone}::before \\{[^}]*content: '([^']+)' / ''`).exec(css);
      expect(m, tone).not.toBeNull();
      marks.add(m![1]);
    }
    expect(marks.size).toBe(5);
  });
});

describe('the old per-page badge code is gone', () => {
  const names = (readdirSync(new URL('../', import.meta.url), { recursive: true }) as string[]).map((n) => n.split(sep).join('/'));
  const sources = names.filter((n) => /\.tsx?$/.test(n) && !/\.test\.tsx?$/.test(n) && !/fixtures\.ts$/.test(n));

  it('has no StateBadge, no Badge module, no stateTone and no statusMark left', () => {
    expect(names).not.toContain('components/Badge.tsx');
    for (const n of sources) {
      expect(readSource(n), n).not.toMatch(/\bStateBadge\b|\bstateTone\b|\bstatusMark\b|components\/Badge'|\.\/Badge'/);
    }
  });
  it('builds a badge class in one component only', () => {
    const builders = sources.filter((n) => /badge badge-|`badge-|\bbadge-\$\{/.test(readSource(n)));
    expect(builders).toEqual(['components/StatusBadge.tsx']);
  });
  it('has no per-status text colour classes left for the property table or the replay verdict', () => {
    for (const n of ['explorer/explorer.css', 'styles.css']) {
      expect(readSource(n), n).not.toMatch(/\.status-(violation|unmet|pass|unrated)\b|\.ok \{/);
    }
    expect(readSource('explorer/PropertyRowView.tsx')).not.toMatch(/status-\$\{/);
    expect(readSource('components/ReplayVerdict.tsx')).not.toMatch(/'error' : 'ok'/);
  });
  it('uses no colour literals in the badge component', () => {
    expect(readSource('components/StatusBadge.tsx')).not.toMatch(/#[0-9a-f]{3,8}\b|rgba?\(/i);
  });
});
