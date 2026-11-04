// @vitest-environment happy-dom
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiError } from '../api';
import type { StartRunBody } from '../startRun';
import type { Target } from '../types';
import { StartRunForm } from './StartRunForm';

const targets: Target[] = [
  { name: 'pubsub-buggy', kind: 'sim', description: 'd', budget: { max_steps: 200_000, max_trials: 200, wall_secs: 60 } },
  { name: 'lock-lease', kind: 'process', description: 'd', budget: { max_steps: 1_000_000, max_trials: 50, wall_secs: 120 } },
];
const ok = (id = 'run_1') => Promise.resolve({ run: { id } });

function setup(start = vi.fn((_: StartRunBody) => ok()), onStarted = vi.fn()) {
  const view = render(<StartRunForm targets={targets} start={start} onStarted={onStarted} />);
  const q = (s: string) => view.container.querySelector(s) as HTMLElement;
  return { ...view, q, start, onStarted };
}
const pick = (c: HTMLElement, label: string) => fireEvent.click(Array.from(c.querySelectorAll('label')).find((l) => l.textContent?.includes(label))!.querySelector('input')!);
const submit = (c: HTMLElement) => fireEvent.submit(c.querySelector('form')!);

describe('StartRunForm', () => {
  afterEach(cleanup);

  it('offers the search and sweep choices with the plan wording, search chosen first', () => {
    const { container } = setup();
    const radios = container.querySelectorAll('input[type="radio"]');
    expect(radios).toHaveLength(2);
    expect((radios[0] as HTMLInputElement).checked).toBe(true);
    expect(container.textContent).toContain('Find a failure (search)');
    expect(container.textContent).toContain('Measure how often (sweep)');
    expect(container.querySelector('fieldset legend')?.textContent).toBe('Run type');
  });

  it('shows the budget the server will enforce for the chosen target, and follows a change of target', () => {
    const { container, q } = setup();
    const budget = () => q('.start-budget').textContent ?? '';
    expect(budget()).toContain('200 trials');
    expect(budget()).toContain('200,000 steps');
    expect(budget()).toContain('60 seconds');
    fireEvent.change(q('select'), { target: { value: 'lock-lease' } });
    expect(budget()).toContain('50 trials');
    expect(budget()).toContain('120 seconds');
    expect(container.textContent).not.toMatch(/proves?\b|guarantee|safe\b|no bugs/i);
  });

  it('shows the step budget only for a simulation target, since only simulations are bounded by it', () => {
    const { q } = setup();
    expect(q('.start-budget').textContent).toContain('Step budget');
    fireEvent.change(q('select'), { target: { value: 'lock-lease' } });
    expect(q('.start-budget').textContent).not.toContain('Step budget');
    expect(q('.start-budget').textContent).not.toContain('steps');
    expect(q('.start-budget').textContent).toContain('Trial budget');
    expect(q('.start-budget').textContent).toContain('Wall-clock budget');
  });

  it('does not throw for a target with no budget: no budget rows, no trials field, and no budget sent', async () => {
    const bare = [{ name: 'old', kind: 'sim', description: 'd' }] as unknown as Target[];
    const start = vi.fn((_: StartRunBody) => ok());
    const onStarted = vi.fn();
    const { container } = render(<StartRunForm targets={bare} start={start} onStarted={onStarted} />);
    expect(container.querySelector('.start-budget')).toBeNull();
    pick(container, 'Measure how often');
    expect(container.querySelector('input[name="trials"]')).toBeNull();
    submit(container);
    await waitFor(() => expect(onStarted).toHaveBeenCalled());
    expect(JSON.stringify(start.mock.calls[0][0])).toBe('{"target":"old","mode":"sweep"}');
  });

  it('explains a search from the docs, and shows no trials field', () => {
    const { container, q } = setup();
    expect(q('.start-explain').textContent).toContain('ends at its first confirmed failure');
    expect(q('.start-explain').textContent).toContain('no rate');
    expect(container.querySelector('input[name="trials"]')).toBeNull();
  });

  it('explains a sweep: same target many times, can take a long time, ends at a budget, and a clean sweep says only that those trials did not fail', () => {
    const { container, q } = setup();
    pick(container, 'Measure how often');
    const t = q('.start-explain').textContent ?? '';
    expect(t).toContain('runs the same target many times');
    expect(t).toContain('can take a long time');
    expect(t).toContain('bounded by the trial budget, the wall-clock budget and, for simulation targets, the step budget');
    expect(t).toContain('A cancel, or three host failures in a row, ends it early');
    expect(t).not.toContain('whichever comes first');
    expect(t).toContain('says only that those trials did not fail');
    expect(container.querySelector('input[name="trials"]')).not.toBeNull();
  });

  it('shows the chosen trials in the budget', () => {
    const { container, q } = setup();
    pick(container, 'Measure how often');
    fireEvent.change(q('input[name="trials"]'), { target: { value: '40' } });
    expect(q('.start-budget').textContent).toContain('40 trials');
  });

  it('sends exactly the search body and goes to the run page', async () => {
    const { container, start, onStarted } = setup();
    submit(container);
    await waitFor(() => expect(onStarted).toHaveBeenCalledWith('/runs/run_1'));
    expect(start).toHaveBeenCalledTimes(1);
    expect(JSON.stringify(start.mock.calls[0][0])).toBe('{"target":"pubsub-buggy","mode":"search"}');
  });

  it('sends exactly the sweep body, with the seed and the whole budget', async () => {
    const { container, q, start, onStarted } = setup(vi.fn((_: StartRunBody) => ok('run_9')));
    fireEvent.change(q('select'), { target: { value: 'lock-lease' } });
    pick(container, 'Measure how often');
    fireEvent.change(q('input[name="seed"]'), { target: { value: '7' } });
    fireEvent.change(q('input[name="trials"]'), { target: { value: '40' } });
    submit(container);
    await waitFor(() => expect(onStarted).toHaveBeenCalledWith('/runs/run_9'));
    expect(JSON.stringify(start.mock.calls[0][0])).toBe(
      '{"target":"lock-lease","seed":7,"mode":"sweep","budget":{"max_steps":1000000,"max_trials":40,"wall_secs":120}}',
    );
  });

  it('refuses bad trials and a bad seed by field, sends nothing, and names the field in the message', () => {
    const { container, q, start } = setup();
    pick(container, 'Measure how often');
    fireEvent.change(q('input[name="trials"]'), { target: { value: '10001' } });
    fireEvent.change(q('input[name="seed"]'), { target: { value: 'x' } });
    submit(container);
    expect(start).not.toHaveBeenCalled();
    expect(q('#trials-error').textContent).toBe('Trials must be between 1 and 10,000');
    expect(q('#seed-error').textContent).toBe('Seed must be a non-negative whole number');
    expect(q('input[name="trials"]').getAttribute('aria-invalid')).toBe('true');
    expect(q('input[name="trials"]').getAttribute('aria-describedby')).toContain('trials-error');
  });

  it('announces each field error, keeps it out of the input label, and ties it to the input', () => {
    const { container, q } = setup();
    pick(container, 'Measure how often');
    fireEvent.change(q('input[name="trials"]'), { target: { value: '007' } });
    fireEvent.change(q('input[name="seed"]'), { target: { value: 'x' } });
    submit(container);
    for (const id of ['trials-error', 'seed-error']) {
      const err = q(`#${id}`);
      expect(err.getAttribute('role')).toBe('alert');
      expect(err.closest('label')).toBeNull();
    }
    expect(q('input[name="trials"]').closest('label')?.textContent).not.toContain('must be');
    expect(q('#trials-error').textContent).toBe('Trials must be a whole number');
  });

  it('moves focus to the first invalid field on a failed submit: the seed before the trials', () => {
    const { container, q } = setup();
    pick(container, 'Measure how often');
    fireEvent.change(q('input[name="trials"]'), { target: { value: '0' } });
    submit(container);
    expect(document.activeElement).toBe(q('input[name="trials"]'));
    fireEvent.change(q('input[name="seed"]'), { target: { value: 'x' } });
    submit(container);
    expect(document.activeElement).toBe(q('input[name="seed"]'));
  });

  it('disables the button while the request is pending, so a second click cannot start a second run', async () => {
    let resolve!: (v: { run: { id: string } }) => void;
    const start = vi.fn((_: StartRunBody) => new Promise<{ run: { id: string } }>((r) => (resolve = r)));
    const { container, q, onStarted } = setup(start);
    submit(container);
    submit(container);
    expect(start).toHaveBeenCalledTimes(1);
    expect((q('button[type="submit"]') as HTMLButtonElement).disabled).toBe(true);
    expect(q('button[type="submit"]').textContent).toBe('Starting…');
    resolve({ run: { id: 'run_2' } });
    await waitFor(() => expect(onStarted).toHaveBeenCalledWith('/runs/run_2'));
  });

  it('shows a refusal by cause, enables the button again and does not navigate', async () => {
    const start = vi.fn((_: StartRunBody) => Promise.reject(new ApiError(403, 'this endpoint requires the run role')));
    const { container, q, onStarted } = setup(start);
    submit(container);
    await waitFor(() => expect(q('[role="alert"]')).not.toBeNull());
    expect(q('[role="alert"]').textContent).toBe('This token cannot start runs. The server says: this endpoint requires the run role.');
    expect((q('button[type="submit"]') as HTMLButtonElement).disabled).toBe(false);
    expect(onStarted).not.toHaveBeenCalled();
  });

  it('cannot start without a target', () => {
    const { container } = render(<StartRunForm targets={[]} start={vi.fn()} onStarted={vi.fn()} />);
    expect((container.querySelector('button[type="submit"]') as HTMLButtonElement).disabled).toBe(true);
    expect(container.querySelector('.start-budget')).toBeNull();
  });

  it('labels the button by what it starts', () => {
    const { container, q } = setup();
    expect(q('button[type="submit"]').textContent).toBe('Start run');
    pick(container, 'Measure how often');
    expect(q('button[type="submit"]').textContent).toBe('Start sweep');
  });
});

describe('StartRunForm access', () => {
  afterEach(cleanup);
  const denied = { canStart: false, note: 'Your role (read) cannot start runs or sweeps. Ask an administrator for the run role.' };

  it('with no access given, behaves as before: enabled, no note', () => {
    const { container } = setup();
    expect((container.querySelector('button[type="submit"]') as HTMLButtonElement).disabled).toBe(false);
    expect(container.querySelector('.start-access-note')).toBeNull();
  });

  it('disables the button and shows the one-line reason, tied to the button for assistive technology', () => {
    const start = vi.fn((_: StartRunBody) => ok());
    const { container } = render(<StartRunForm targets={targets} start={start} onStarted={vi.fn()} access={denied} />);
    const button = container.querySelector('button[type="submit"]') as HTMLButtonElement;
    const note = container.querySelector('.start-access-note') as HTMLElement;
    expect(button.disabled).toBe(true);
    expect(note.textContent).toBe(denied.note);
    expect(button.getAttribute('aria-describedby')).toBe(note.id);
  });

  it('does not call the server when a submit arrives while access is denied', () => {
    const start = vi.fn((_: StartRunBody) => ok());
    const { container } = render(<StartRunForm targets={targets} start={start} onStarted={vi.fn()} access={denied} />);
    submit(container);
    expect(start).not.toHaveBeenCalled();
  });

  it('still starts when access is allowed', async () => {
    const start = vi.fn((_: StartRunBody) => ok());
    const { container } = render(<StartRunForm targets={targets} start={start} onStarted={vi.fn()} access={{ canStart: true }} />);
    submit(container);
    await waitFor(() => expect(start).toHaveBeenCalledTimes(1));
  });
});
