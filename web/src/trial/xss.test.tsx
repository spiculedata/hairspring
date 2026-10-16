import { renderToString } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { CH } from '../guest/chars';
import { assertTextOnly, CORPUS } from '../guest/hostile';
import { detail, proc, property, record, schedule, trialRow, witness } from './fixtures';
import { TrialView } from './TrialView';

/** A trial in which every string that comes from the program under test is the hostile text. */
function hostileDetail(h: string) {
  const w = witness({ proc: h, thread: h });
  const p = proc({
    tag: h,
    sdk: h,
    sdk_version: h,
    catalog_hash: h,
    threads: [{ thread: h, evals: 2410, tape_consumed: 0, key_stable: false }],
  });
  const props = property({ id: h, msg: h, loc: { file: h, line: 1, func: h }, first_false: w });
  return detail({
    trial: trialRow({ guest: { reason: h, failing_ids: [h] } }),
    guest: {
      detail: record({ procs: [p], properties: [props], conflicts: [{ id: h, proc: h, kept: h, rejected: h }] }),
      schedule: schedule({ virttime_elapsed: h, timeslice_stats: h, per_thread_timeslice: h }),
    },
  });
}

describe.each(CORPUS)('failure page XSS corpus: %s', (_name, hostile) => {
  const html = renderToString(
    <TrialView
      detail={hostileDetail(hostile)}
      context={{ stoppedAtFirstFailure: false, completionLabel: 'complete' }}
      tails={{ stdout: { status: 'ready', text: `${hostile}\n${hostile}` }, stderr: { status: 'error', error: hostile } }}
      onDownload={() => undefined}
    />,
  );

  it('shows every guest string as escaped text only: no element, attribute or raw control character from it', () => {
    assertTextOnly(html);
  });

  it('stays bounded however large the strings were', () => {
    expect(html.length).toBeLessThan(600_000);
  });
});

describe('failure page: escaped output is visible', () => {
  it('shows a bidi override and an ANSI escape in output as visible escapes and the markup as text', () => {
    const html = renderToString(
      <TrialView
        detail={detail()}
        context={undefined}
        tails={{ stdout: { status: 'ready', text: `${CH.ESC}[31m<b>red</b>${CH.RLO}\n` }, stderr: { status: 'absent' } }}
        onDownload={() => undefined}
      />,
    );
    expect(html).toContain('\\u{1B}');
    expect(html).toContain('\\u{202E}');
    expect(html).toContain('&lt;b&gt;red&lt;/b&gt;');
  });
});
