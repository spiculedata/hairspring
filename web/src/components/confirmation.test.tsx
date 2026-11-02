import { renderToString } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { contrastRatio } from '../contrast';
import { readSource } from '../readSource';
import { CHECK_STATUSES, FINDING_STATES, checkStatusEntry, findingEntry } from '../statusMap';
import type { CheckView } from '../types';
import { CheckTable, environmentText } from './CheckTable';
import { ConfirmationLadder } from './ConfirmationLadder';
import { StatusBadge } from './StatusBadge';

const text = (html: string): string => html.replace(/<[^>]+>/g, '');
const FINDINGS = ['candidate', 'rerun_repeated', 'confirmed', 'verified', 'not_reproducible', 'control_failed'];

function check(over: Partial<CheckView> = {}): CheckView {
  return {
    id: 'c1',
    confirmation_id: 'conf_a',
    ordinal: 1,
    kind: 'same_seed_rerun',
    status: 'pass',
    counts: { passed: 3, failed: 0, inconclusive: 0 },
    artefacts: [],
    environment: {},
    started_at: Date.UTC(2026, 10, 2, 9, 0, 0),
    finished_at: Date.UTC(2026, 10, 2, 9, 1, 0),
    runner_trusted: true,
    created_at: Date.UTC(2026, 10, 2, 9, 1, 0),
    guest: { command: 'hermit run', detail: {} },
    ...over,
  };
}

/** The text of one column's cell in each body row, found by its heading. */
function columnValues(html: string, heading: string): string[] {
  const heads = (html.match(/<th[^>]*>.*?<\/th>/g) ?? []).map(text);
  const col = heads.indexOf(heading);
  const rows = html.match(/<tbody>.*<\/tbody>/)?.[0].match(/<tr>.*?<\/tr>/g) ?? [];
  return rows.map((r) => text((r.match(/<td[^>]*>.*?<\/td>/g) ?? [])[col] ?? ''));
}

describe('status map for findings and checks', () => {
  it.each([
    ['candidate', 'Candidate', 'idle'],
    ['rerun_repeated', 'Rerun repeated', 'idle'],
    ['confirmed', 'Confirmed', 'confirm'],
    ['verified', 'Verified', 'confirm'],
    ['not_reproducible', 'Not reproducible', 'idle'],
    ['control_failed', 'Control failed', 'idle'],
  ])('maps finding %s to "%s" in the %s tone', (state, label, tone) => {
    expect(findingEntry(state)).toEqual({ label, tone });
    expect(Object.keys(FINDING_STATES)).toContain(state);
  });
  it.each([
    ['pass', 'Pass', 'good'],
    ['fail', 'Fail', 'bad'],
    ['inconclusive', 'Inconclusive', 'idle'],
    ['skipped', 'Skipped', 'idle'],
  ])('maps check %s to "%s" in the %s tone', (status, label, tone) => {
    expect(checkStatusEntry(status)).toEqual({ label, tone });
    expect(Object.keys(CHECK_STATUSES)).toContain(status);
  });
  it('shows an unknown word as a neutral "Unknown", never in a pass or confirmation tone', () => {
    for (const w of ['', 'ok', 'PASS', 'confirm', 'reproduced', 'toString', '__proto__']) {
      expect(findingEntry(w)).toEqual({ label: 'Unknown', tone: 'idle' });
      expect(checkStatusEntry(w)).toEqual({ label: 'Unknown', tone: 'idle' });
    }
  });
  it('keeps the server word in the tooltip and escapes it', () => {
    const html = renderToString(<StatusBadge kind="finding" status={'"><img src=x onerror=1>'} />);
    expect(html).not.toContain('<img');
    expect(renderToString(<StatusBadge kind="finding" status="rerun_repeated" />)).toContain('title="Server state: rerun_repeated"');
    expect(renderToString(<StatusBadge kind="check" status="inconclusive" />)).toContain('title="Server status: inconclusive"');
  });
  it('draws the confirmation tone from tokens that pass 4.5:1 in both themes, with its own mark', () => {
    const css = readSource('components/confirmation.css');
    expect(css).toMatch(/\.badge-confirm \{[^}]*color: var\(--confirm\); background: var\(--confirm-bg\)/);
    expect(css).toMatch(/\.badge-confirm::before \{[^}]*content: '◆' \/ ''/);
    const styles = readSource('styles.css');
    const dark = styles.slice(styles.indexOf('@media (prefers-color-scheme: dark)'));
    const lightBlock = styles.slice(0, styles.indexOf('@media (prefers-color-scheme: dark)'));
    for (const block of [lightBlock, dark]) {
      const fg = /--confirm: (#[0-9a-f]{6})/i.exec(block)?.[1];
      const bg = /--confirm-bg: (#[0-9a-f]{6})/i.exec(block)?.[1];
      expect(contrastRatio(fg!, bg!)).toBeGreaterThanOrEqual(4.5);
    }
  });
  it('has no colour literals in the new stylesheet or components', () => {
    for (const f of ['components/confirmation.css', 'components/ConfirmationLadder.tsx', 'components/CheckTable.tsx']) {
      expect(readSource(f), f).not.toMatch(/#[0-9a-f]{3,8}\b|rgba?\(/i);
    }
  });
});

describe('ConfirmationLadder', () => {
  it.each([
    ['candidate', 0],
    ['rerun_repeated', 1],
    ['confirmed', 2],
    ['verified', 3],
  ])('marks %s as the current rung and the lower rungs as reached', (state, index) => {
    const html = renderToString(<ConfirmationLadder finding={state} environmentPinned={false} />);
    const items = html.match(/<li[^>]*>.*?<\/li>/g) ?? [];
    expect(items).toHaveLength(4);
    items.forEach((li, i) => {
      const mark = i === index ? 'Current' : i < index ? 'Reached' : 'Not reached';
      expect(text(li), `${state} rung ${i}`).toContain(mark);
      expect(li.includes('aria-current="step"')).toBe(i === index);
    });
  });
  it.each([
    ['not_reproducible', 'A rerun or the replay did not repeat the failure on the environment it was recorded in.'],
    ['control_failed', 'The reruns repeat the failure, but the fixed build still fails at this seed.'],
  ])('shows %s as an end of the ladder with no rung marked', (state, meaning) => {
    const html = renderToString(<ConfirmationLadder finding={state} environmentPinned={false} />);
    expect(text(html)).toContain(meaning);
    expect(text(html)).toContain('an end of the ladder, not a rung');
    expect(html).not.toContain('aria-current');
    expect(html).not.toMatch(/Current|Reached/);
    expect(html).not.toContain('badge-confirm');
  });
  it('says confirmed and verified with the confirmation tone, and whether the environment is pinned', () => {
    const pinned = renderToString(<ConfirmationLadder finding="confirmed" environmentPinned />);
    expect(pinned).toContain('badge-confirm');
    expect(text(pinned)).toContain('Environment pinned');
    const loose = renderToString(<ConfirmationLadder finding="verified" environmentPinned={false} />);
    expect(text(loose)).toContain('Environment not pinned');
  });
  it('does not mention the environment for a state that is not confirmed or verified', () => {
    for (const s of ['candidate', 'rerun_repeated', 'not_reproducible', 'control_failed']) {
      expect(text(renderToString(<ConfirmationLadder finding={s} environmentPinned />))).not.toMatch(/Environment (not )?pinned/);
    }
  });
  it('never uses the confirmation tone or the word confirmed below the confirmed rung', () => {
    for (const s of ['candidate', 'rerun_repeated']) {
      const html = renderToString(<ConfirmationLadder finding={s} environmentPinned />);
      expect(html).not.toContain('badge-confirm');
      expect(html).not.toContain('badge-good');
    }
  });
  it('shows an unknown word as sent, escaped, neutral, with no position and no pass tone', () => {
    const html = renderToString(<ConfirmationLadder finding={'<b>fixed</b>'} environmentPinned />);
    expect(html).not.toContain('<b>');
    expect(text(html)).toContain('Unknown');
    expect(text(html)).toContain('no wording for');
    expect(text(html)).toContain('<b>fixed</b>'.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;'));
    expect(html).toContain('badge-idle');
    expect(html).not.toMatch(/badge-(good|confirm)|aria-current|Current|Reached/);
  });
  it('says no state means the failure is fixed', () => {
    for (const s of FINDINGS) {
      const t = text(renderToString(<ConfirmationLadder finding={s} environmentPinned />));
      expect(t).toContain('None of the states says the failure is fixed.');
      expect(t).not.toContain('how far the checks have repeated');
      expect(t).not.toMatch(/\breproduced\b|\bresolved\b/i);
    }
  });
});

describe('CheckTable', () => {
  it.each([
    ['same_seed_rerun', 'Same-seed rerun'],
    ['bundle_replay', 'Bundle replay'],
    ['fixed_control', 'Fixed control'],
    ['strict_verify', 'Strict verify'],
  ])('labels the kind %s as "%s"', (kind, label) => {
    expect(text(renderToString(<CheckTable checks={[check({ kind: kind as CheckView['kind'] })]} confirmationId="conf_a" />))).toContain(label);
  });
  it.each([
    ['pass', 'Pass', 'badge-good'],
    ['fail', 'Fail', 'badge-bad'],
    ['inconclusive', 'Inconclusive', 'badge-idle'],
    ['skipped', 'Skipped', 'badge-idle'],
  ])('shows the status %s as "%s"', (status, label, cls) => {
    const html = renderToString(<CheckTable checks={[check({ status: status as CheckView['status'] })]} confirmationId="conf_a" />);
    expect(html).toContain(`>${label}<`);
    expect(html).toContain(cls);
  });
  it('shows counts exactly as given', () => {
    const html = renderToString(<CheckTable checks={[check({ counts: { passed: 2, failed: 1, inconclusive: 4 } })]} confirmationId="conf_a" />);
    expect(text(html)).toContain('2 passed, 1 failed, 4 inconclusive');
  });
  it('shows an inconclusive check in a neutral tone with the environment note, never as a fail or a pass', () => {
    const c = check({
      status: 'inconclusive',
      counts: { passed: 0, failed: 0, inconclusive: 3 },
      guest: { command: 'x', detail: { inconclusive: 'environment differs: libc changed' } },
    });
    const html = renderToString(<CheckTable checks={[c]} confirmationId="conf_a" />);
    expect(html).toContain('badge-idle');
    expect(html).not.toMatch(/badge-(good|bad)/);
    expect(text(html)).toContain('Environment differs from the recording');
    expect(text(html)).toContain('inconclusive: environment differs from the recording; this does not show the bug is fixed');
  });
  it('reads the environment from the replay key and says nothing when the runner did not report', () => {
    expect(environmentText({ environment_changed: 'true' })).toBe('Environment differs from the recording');
    expect(environmentText({ environment_changed: 'false' })).toBe('Environment matches the recording');
    expect(environmentText({})).toBe('Not reported');
    expect(environmentText({ environment_changed: 'maybe' })).toBe('Not reported');
  });
  it('does not show the environment note when no inconclusive check differs', () => {
    const html = renderToString(<CheckTable checks={[check({ guest: { command: 'x', detail: { environment_changed: 'false' } } })]} confirmationId="conf_a" />);
    expect(text(html)).not.toContain('this does not show the bug is fixed');
  });
  it('shows Yes or No in the "Counts towards the finding" cell of each row, for a mixed list', () => {
    const checks = [
      check({ id: 'a', ordinal: 1 }),
      check({ id: 'b', confirmation_id: 'conf_old', ordinal: 2 }),
      check({ id: 'c', ordinal: 3 }),
      check({ id: 'd', confirmation_id: 'conf_old', ordinal: 4 }),
    ];
    const html = renderToString(<CheckTable checks={checks} confirmationId="conf_a" />);
    expect(columnValues(html, 'Counts towards the finding')).toEqual(['Yes', 'No', 'Yes', 'No']);
    expect(text(html)).toContain('Checks from other confirmations are listed but do not count towards it.');
  });
  it('shows no other-confirmations note when every check is in the finding confirmation', () => {
    const html = renderToString(<CheckTable checks={[check()]} confirmationId="conf_a" />);
    expect(columnValues(html, 'Counts towards the finding')).toEqual(['Yes']);
    expect(text(html)).not.toContain('other confirmations');
  });
  it('makes no Counts-towards claim, and no other-confirmations note, when the server names no confirmation', () => {
    const html = renderToString(<CheckTable checks={[check(), check({ id: 'z', ordinal: 2 })]} confirmationId={null} />);
    expect(text(html)).not.toContain('Counts towards');
    expect(text(html)).not.toContain('other confirmations');
    expect(html.match(/<tbody>.*<\/tbody>/)?.[0].match(/<tr>/g)).toHaveLength(2);
  });
  it('shows trusted and untrusted runners in words, per row', () => {
    const html = renderToString(<CheckTable checks={[check({ runner_trusted: false }), check({ id: 'b', ordinal: 2 })]} confirmationId="conf_a" />);
    expect(columnValues(html, 'Trusted runner')).toEqual(['No', 'Yes']);
  });
  it('says some counted checks came from untrusted runners only when a counting check did', () => {
    const line = 'Some checks counted here came from runners that are not trusted.';
    const untrusted = renderToString(<CheckTable checks={[check({ runner_trusted: false })]} confirmationId="conf_a" />);
    expect(text(untrusted)).toContain(line);
    const trusted = renderToString(<CheckTable checks={[check()]} confirmationId="conf_a" />);
    expect(text(trusted)).not.toContain(line);
    const outside = renderToString(
      <CheckTable checks={[check(), check({ id: 'o', confirmation_id: 'conf_old', runner_trusted: false })]} confirmationId="conf_a" />,
    );
    expect(text(outside)).not.toContain(line);
    const unnamed = renderToString(<CheckTable checks={[check({ runner_trusted: false })]} confirmationId={null} />);
    expect(text(unnamed)).not.toContain(line);
  });
  it('puts the note on what Pass means above the table, and names the headings', () => {
    const html = renderToString(<CheckTable checks={[check()]} confirmationId="conf_a" />);
    const note = 'Pass means the failure repeated';
    expect(text(html)).toContain(note);
    expect(html.indexOf(note)).toBeLessThan(html.indexOf('<table'));
    expect(html).toContain('>Check result<');
    expect(html).toContain('>Runs counted<');
  });
  it('keeps table semantics: column scopes, a hidden caption, and a scroll wrapper', () => {
    const html = renderToString(<CheckTable checks={[check()]} confirmationId="conf_a" />);
    expect((html.match(/<th scope="col"/g) ?? []).length).toBe((html.match(/<th[ >]/g) ?? []).length);
    expect(html).toMatch(/<caption class="sr-only">/);
    expect(html).toContain('class="checks-scroll"');
    expect(readSource('components/confirmation.css')).not.toMatch(/display:\s*block/);
  });
  it('shows an unknown kind and status as sent, escaped, with a neutral badge', () => {
    const html = renderToString(
      <CheckTable checks={[check({ kind: 'new_kind' as CheckView['kind'], status: 'weird' as CheckView['status'] })]} confirmationId="conf_a" />,
    );
    expect(text(html)).toContain('new_kind');
    expect(text(html)).toContain('weird');
    expect(text(html)).toContain('Unknown');
    expect(html).toContain('badge-idle');
    expect(html).not.toMatch(/badge-(good|confirm)/);
  });
  it('escapes hostile text in the command and the notes', () => {
    const hostile = '<img src=x onerror=alert(1)>';
    const html = renderToString(
      <CheckTable checks={[check({ guest: { command: hostile, detail: { [hostile]: hostile } } })]} confirmationId="conf_a" />,
    );
    expect(html).not.toContain('<img');
    expect(html).not.toContain('onerror=alert(1)>');
    expect(html).toContain('&lt;img');
  });
  it('shows control characters as visible escapes', () => {
    const html = renderToString(<CheckTable checks={[check({ guest: { command: 'a\u001b[31mb‮c', detail: {} } })]} confirmationId="conf_a" />);
    expect(html).not.toContain('\u001b');
    expect(html).not.toContain('‮');
  });
  it('never reads as a pass for unknown, inconclusive or skipped results', () => {
    for (const status of ['inconclusive', 'skipped', 'x', '']) {
      const html = renderToString(<CheckTable checks={[check({ status: status as CheckView['status'] })]} confirmationId="conf_a" />);
      expect(html, status).not.toContain('badge-good');
      expect(html, status).not.toContain('>Pass<');
    }
  });
});
