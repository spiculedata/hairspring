import { renderToString } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { detail, HEALTH_CLEAN, proc, property, record, schedule, trialRow } from './fixtures';
import { TrialView, type TrialViewProps } from './TrialView';

const text = (html: string): string => html.replace(/<[^>]+>/g, ' ').replace(/&#x27;/g, "'").replace(/\s+/g, ' ');

const base: Omit<TrialViewProps, 'detail'> = {
  context: { stoppedAtFirstFailure: false, completionLabel: 'complete' },
  tails: { stdout: { status: 'ready', text: 'line one\nline two\n' }, stderr: { status: 'ready', text: '' } },
  onDownload: () => undefined,
};

function view(over: Partial<TrialViewProps> = {}): string {
  return renderToString(<TrialView detail={detail()} {...base} {...over} />);
}

describe('TrialView header', () => {
  it('says the seed failed, names the failing property and keeps the reason as guest text', () => {
    const t = text(view());
    expect(t).toContain('Seed 1016 failed');
    expect(t).toContain('wal.no-lost-commit');
    expect(t).toContain('always wal.no-lost-commit was false 1 time(s)');
    expect(t).toMatch(/wal\.c\s*:2231/);
  });

  it('shows the label chip and, for a worker that is not trusted, the unverified chip', () => {
    expect(text(view())).toContain('label not declared');
    expect(text(view())).not.toContain('unverified worker data');
    const untrusted = detail({ trial: trialRow({ worker_trusted: false }) });
    expect(text(view({ detail: untrusted }))).toContain('unverified worker data');
  });

  it('says what it does not do: no replay claim, no reproduction claim', () => {
    const t = text(view());
    expect(t).toContain('This page does not re-run anything');
    expect(t).not.toMatch(/reproduced|\bfound\b|discovered|unassisted|flaky/i);
  });

  it('says a run that stopped at its first failure gives no rate', () => {
    const t = text(view({ context: { stoppedAtFirstFailure: true, completionLabel: 'stopped at first failure' } }));
    expect(t).toContain('stopped at its first failure');
    expect(t).toContain('not a rate');
    expect(text(view())).not.toContain('not a rate');
  });

  it('says a perturbing trial is excluded from tier claims, and shows the exit status and signal', () => {
    const t = text(view({ detail: detail({ trial: trialRow({ perturbing: true, exit_code: null, signal: 11 }) }) }));
    expect(t).toContain('eval lines perturb the schedule');
    expect(t).toContain('signal 11');
  });

  it('links back to the run', () => {
    expect(view()).toContain('<a href="/runs/run_0123456789abcdef">');
  });
});

describe('TrialView witness', () => {
  it('reads as a thread own k-th evaluation, with the thread key stability and the four values', () => {
    const t = text(view());
    expect(t).toContain('writer#1');
    expect(t).toContain('evaluation number 2,301 (its 2,301st) of the 2,410 that thread made');
    expect(t).toContain('stable key');
    expect(t).toContain('3000 2999 0 0');
    expect(t).toContain('proc main');
  });

  it('says so when a failing property carries no witness', () => {
    const d = detail({ guest: { detail: record({ properties: [property({ first_false: null })] }), schedule: schedule() } });
    expect(text(view({ detail: d }))).toContain('no witness was recorded');
  });

  it('says nothing failed when no kept property did', () => {
    const d = detail({ trial: trialRow({ verdict: 'pass' }), guest: { detail: record({ properties: [property({ falses: 0, first_false: null })] }), schedule: null } });
    expect(text(view({ detail: d }))).toContain('No kept property failed in this trial.');
  });
});

describe('TrialView thread lanes', () => {
  it('has a text alternative: a table with a row and counts for every thread, and says lanes are per thread and not aligned in time', () => {
    const html = view();
    const t = text(html);
    expect(html).toMatch(/<table[^>]*aria-label="Threads"/);
    expect(t).toContain('2,410');
    expect(t).toContain('1,120');
    expect(t).toContain('unstable key');
    expect(t).toContain('not aligned in time');
    expect(t).toContain('No order across threads is recorded');
  });

  it('marks the witness with words as well as a glyph, so meaning is not colour or shape alone', () => {
    const t = text(view());
    expect(t).toContain('witness at evaluation 2,301');
  });

  it('shows only the first lanes of a very large trial and offers the rest', () => {
    const threads = Array.from({ length: 700 }, (_, i) => ({ thread: `t${i}`, evals: i, tape_consumed: 0, key_stable: true }));
    const d = detail({ guest: { detail: record({ procs: [proc({ threads })] }), schedule: null } });
    const html = view({ detail: d });
    expect((html.match(/<tr/g) ?? []).length).toBeLessThan(400);
    expect(text(html)).toContain('Show more threads');
  });
});

describe('TrialView processes and health', () => {
  it('lists each process with its sdk, end state and work', () => {
    const t = text(view());
    expect(t).toContain('ended: exit, status 0');
    expect(t).toContain('9f2e41');
    expect(t).toContain('3,000');
  });

  it('flags a process with no end report', () => {
    const d = detail({ guest: { detail: record({ procs: [proc({ end: null })] }), schedule: null } });
    expect(text(view({ detail: d }))).toContain('no end report');
  });

  it('reports clean health in words and lists each loss counter when degraded', () => {
    expect(text(view())).toContain('accepted 211');
    expect(text(view())).toContain('degraded: no');
    const d = detail({
      trial: trialRow({ degraded: true }),
      guest: { detail: record({ health: { ...HEALTH_CLEAN, malformed: 2 } }), schedule: null },
    });
    const t = text(view({ detail: d }));
    expect(t).toContain('degraded: yes');
    expect(t).toContain('2 malformed event lines');
  });
});

describe('TrialView schedule', () => {
  it('shows the deterministic scheduler fields and the fingerprint', () => {
    const t = text(view());
    expect(t).toContain('22,979');
    expect(t).toContain('21,739');
    expect(t).toContain('4c16f00dcafe0001');
  });

  it('says plainly when no scheduler summary was kept, and still shows what the row holds', () => {
    const t = text(view({ detail: detail({ guest: { detail: record(), schedule: null } }) }));
    expect(t).toContain('No scheduler summary was kept');
    expect(t).toContain('4c16f00dcafe0001');
  });
});

describe('TrialView detail that was not kept', () => {
  it('says so, and shows no lanes, processes or witness', () => {
    const d = detail({ trial: trialRow({ kept: 'none', has_detail: false, has_output: false }), guest: { detail: null, schedule: null }, output_streams: [] });
    const t = text(view({ detail: d, tails: {} }));
    expect(t).toContain('No detail was kept for this trial');
    expect(t).not.toContain('Threads');
  });

  it('says when the kept property list was left out for size', () => {
    const d = detail({ guest: { detail: { ...record(), truncated: true }, schedule: null } });
    expect(text(view({ detail: d }))).toContain('The property list was too large to keep');
  });
});

describe('TrialView output', () => {
  it('shows each kept stream as escaped lines, with a download button and the raw-events note', () => {
    const html = view();
    const t = text(html);
    expect(t).toContain('line one');
    expect(t).toContain('line two');
    expect(html).toMatch(/<button[^>]*>Download stdout<\/button>/);
    expect(t).toContain('Raw event lines are not stored');
  });

  it('says how many bytes the capture cut before it was kept', () => {
    const d = detail({ output_omitted_bytes: { stdout: 1234, stderr: 0 } });
    expect(text(view({ detail: d }))).toContain('1,234 bytes cut from the start before it was kept');
  });

  it('bounds a 10 MB tail and says how much it left out', () => {
    const big = 'y'.repeat(10 * 1024 * 1024);
    const html = view({ tails: { stdout: { status: 'ready', text: big }, stderr: { status: 'absent' } } });
    expect(html.length).toBeLessThan(200_000);
    expect(text(html)).toMatch(/earlier characters? not shown here; download the stream/);
  });

  it('shows loading and error states without breaking the page', () => {
    const t = text(view({ tails: { stdout: { status: 'loading' }, stderr: { status: 'error', error: 'HTTP 500' } } }));
    expect(t).toContain('Loading output');
    expect(t).toContain('HTTP 500');
  });
});
