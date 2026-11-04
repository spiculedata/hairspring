import { renderToString } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { PER_THREAD_NOTE, RAW_EVENTS_NOTE } from '../wording';
import { detail, proc, record, schedule, trialRow } from './fixtures';
import { TrialView, type TrialViewProps } from './TrialView';

const text = (html: string): string => html.replace(/<[^>]+>/g, ' ').replace(/&#x27;/g, "'").replace(/\s+/g, ' ');
const count = (haystack: string, needle: string): number => haystack.split(needle).length - 1;

const base: Omit<TrialViewProps, 'detail'> = {
  context: { stoppedAtFirstFailure: false, completionLabel: 'complete' },
  tails: { stdout: { status: 'ready', text: 'line one\nline two\n' }, stderr: { status: 'ready', text: 'oops\n' } },
  onDownload: () => undefined,
};
const view = (over: Partial<TrialViewProps> = {}): string => renderToString(<TrialView detail={detail()} {...base} {...over} />);

const everything = () =>
  detail({ trial: trialRow({ perturbing: true }), guest: { detail: { ...record(), truncated: true }, schedule: schedule() } });
const stopped = { stoppedAtFirstFailure: true, completionLabel: 'stopped at first failure' };

describe('TrialView navigation', () => {
  it('has an in-page list of links and a select for phones, both naming the sections', () => {
    const html = view();
    expect(html).toMatch(/<nav[^>]*aria-label="On this page"/);
    expect(html).toMatch(/<button[^>]*data-target="threads-title"[^>]*>Threads<\/button>/);
    expect(html).not.toContain('href="#');
    expect(html).toMatch(/<select[^>]*aria-label="Jump to section"/);
    expect(html).toMatch(/<option[^>]*value="output-title"/);
  });

  it('gives every nav target an element with that id', () => {
    const html = view();
    for (const id of ['witness-title', 'threads-title', 'procs-title', 'health-title', 'schedule-title', 'output-title', 'about-title']) {
      expect(html).toContain(`id="${id}"`);
    }
  });
});

describe('TrialView ranked caveat', () => {
  const caveats = (html: string): number => count(html, 'class="note caveat"');

  it('shows no ranked caveat for an ordinary trial', () => {
    expect(caveats(view())).toBe(0);
  });

  it('shows exactly one caveat when several apply, the first-failure one outranking the others', () => {
    const html = view({ detail: everything(), context: stopped });
    expect(caveats(html)).toBe(1);
    const at = html.indexOf('class="note caveat"');
    expect(text(html.slice(at, html.indexOf('</p>', at)))).toContain('stopped at its first failure');
  });

  it('keeps the caveats that were not ranked first on the page, under About this page', () => {
    const html = view({ detail: everything(), context: stopped });
    const about = text(html.slice(html.indexOf('id="about-title"')));
    expect(about).toContain('eval lines perturb the schedule');
    expect(about).toContain('The property list was too large to keep');
    expect(about).toContain('This page does not re-run anything');
  });

  it('keeps every honesty sentence on the page for a trial with everything to say', () => {
    const t = text(view({ detail: everything(), context: stopped }));
    for (const s of ['not a rate', 'eval lines perturb the schedule', 'too large to keep', 'does not re-run anything', 'not aligned in time', 'longer lane does not mean more time']) {
      expect(t).toContain(s);
    }
    expect(t).toContain(RAW_EVENTS_NOTE);
  });
});

describe('TrialView lanes', () => {
  it('says lanes are not aligned in time once', () => {
    expect(count(text(view()), 'not aligned in time')).toBe(1);
    expect(count(text(view()), PER_THREAD_NOTE)).toBe(1);
  });

  it('labels each lane with its own scale', () => {
    const t = text(view());
    expect(t).toContain('0 to 2,410 evaluations');
    expect(t).toContain('0 to 1,120 evaluations');
    expect(t).toContain('0 to 210 evaluations');
  });

  it('labels a thread with no evaluations as such rather than a zero-width scale', () => {
    const d = detail({ guest: { detail: record({ procs: [proc({ threads: [{ thread: 'idle', evals: 0, tape_consumed: 0, key_stable: true }] })] }), schedule: null } });
    expect(text(view({ detail: d }))).toContain('no evaluations');
  });
});

describe('TrialView schedule fields', () => {
  const withSchedule = (over: Parameters<typeof schedule>[0]) => detail({ guest: { detail: record(), schedule: schedule(over) } });

  it('shows the parsed timeslice count and total as labelled fields', () => {
    const t = text(view({ detail: withSchedule({ timeslice_stats: '{"count":3,"sum_ns":1500}' }) }));
    expect(t).toMatch(/Timeslices\s+3/);
    expect(t).toContain('1,500 ns');
  });

  it('falls back to the reported text when the statistics are not in the expected shape', () => {
    expect(text(view({ detail: withSchedule({ timeslice_stats: 'odd shape' }) }))).toContain('odd shape');
  });

  it('shows per-thread timeslices as a table when they parse', () => {
    const html = view({ detail: withSchedule({ per_thread_timeslice: '[{"thread":"w","count":4}]' }) });
    expect(html).toMatch(/<table[^>]*aria-label="Per-thread timeslices"/);
  });
});

describe('TrialView output viewer', () => {
  it('numbers the lines shown', () => {
    const html = view();
    expect(html).toMatch(/class="ln"[^>]*>1</);
    expect(html).toMatch(/class="ln"[^>]*>2</);
  });

  it('offers a find box and a switch between the kept streams', () => {
    const html = view();
    expect(html).toMatch(/<input[^>]*aria-label="Find in output"/);
    expect(html).toMatch(/role="tab"[^>]*>stdout</);
    expect(html).toMatch(/role="tab"[^>]*>stderr</);
  });
});
