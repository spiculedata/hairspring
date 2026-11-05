// @vitest-environment happy-dom
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { FailureFilterBar, RunFilterBar } from './FilterBars';
import { MAX_TARGET_BYTES } from './filters';

afterEach(cleanup);

const RUN = 'run_0123456789abcdef';

describe('RunFilterBar', () => {
  it('is a labelled search region with a labelled control for each filter', () => {
    render(<RunFilterBar filters={{}} onChange={() => undefined} />);
    expect(screen.getByRole('search', { name: 'Filter runs' })).toBeTruthy();
    for (const name of ['State', 'Run type', 'Target']) expect(screen.getByLabelText(name)).toBeTruthy();
  });

  it('offers every state the server stores, with the list’s own words, and no wording that claims a pass', () => {
    render(<RunFilterBar filters={{}} onChange={() => undefined} />);
    const options = Array.from((screen.getByLabelText('State') as HTMLSelectElement).options).map((o) => [o.value, o.textContent]);
    expect(options).toEqual([
      ['', 'All'],
      ['queued', 'Queued'],
      ['running', 'Running'],
      ['passed', 'Finished'],
      ['failed', 'Failed'],
      ['error', 'Error'],
      ['cancelled', 'Cancelled'],
    ]);
    expect(document.body.textContent).not.toMatch(/no failure seen|passed/i);
  });

  it('offers the two run types with the start form’s words', () => {
    render(<RunFilterBar filters={{}} onChange={() => undefined} />);
    const options = Array.from((screen.getByLabelText('Run type') as HTMLSelectElement).options).map((o) => [o.value, o.textContent]);
    expect(options).toEqual([
      ['', 'All'],
      ['search', 'Find a failure (search)'],
      ['sweep', 'Measure how often (sweep)'],
    ]);
  });

  it('applies a state or a run type as soon as it is chosen, keeping the other filters', () => {
    const onChange = vi.fn();
    render(<RunFilterBar filters={{ target: 'lock-lease' }} onChange={onChange} />);
    fireEvent.change(screen.getByLabelText('State'), { target: { value: 'failed' } });
    expect(onChange).toHaveBeenLastCalledWith({ target: 'lock-lease', state: 'failed' });
    fireEvent.change(screen.getByLabelText('Run type'), { target: { value: 'sweep' } });
    expect(onChange).toHaveBeenLastCalledWith({ target: 'lock-lease', mode: 'sweep' });
  });

  it('clears a state by choosing All', () => {
    const onChange = vi.fn();
    render(<RunFilterBar filters={{ state: 'failed' }} onChange={onChange} />);
    fireEvent.change(screen.getByLabelText('State'), { target: { value: '' } });
    expect(onChange).toHaveBeenLastCalledWith({ state: undefined });
  });

  it('applies a typed target on Enter, and does not ask again for the same value', () => {
    const onChange = vi.fn();
    render(<RunFilterBar filters={{}} onChange={onChange} />);
    const box = screen.getByLabelText('Target');
    fireEvent.change(box, { target: { value: 'lock-lease' } });
    expect(onChange).not.toHaveBeenCalled();
    fireEvent.keyDown(box, { key: 'Enter' });
    expect(onChange).toHaveBeenCalledTimes(1);
    expect(onChange).toHaveBeenLastCalledWith({ target: 'lock-lease' });
  });

  it('applies a typed target when the box loses focus', () => {
    const onChange = vi.fn();
    render(<RunFilterBar filters={{ state: 'queued' }} onChange={onChange} />);
    const box = screen.getByLabelText('Target');
    fireEvent.change(box, { target: { value: 'pubsub-buggy' } });
    fireEvent.blur(box);
    expect(onChange).toHaveBeenLastCalledWith({ state: 'queued', target: 'pubsub-buggy' });
  });

  it('does not call back when the box loses focus with the value already in force', () => {
    const onChange = vi.fn();
    render(<RunFilterBar filters={{ target: 'lock-lease' }} onChange={onChange} />);
    fireEvent.blur(screen.getByLabelText('Target'));
    expect(onChange).not.toHaveBeenCalled();
  });

  it('removes the target filter when the box is emptied', () => {
    const onChange = vi.fn();
    render(<RunFilterBar filters={{ target: 'lock-lease' }} onChange={onChange} />);
    const box = screen.getByLabelText('Target');
    fireEvent.change(box, { target: { value: '' } });
    fireEvent.blur(box);
    expect(onChange).toHaveBeenLastCalledWith({ target: undefined });
  });

  it('says what is wrong with a target the server would refuse, marks the box, and sends nothing', () => {
    const onChange = vi.fn();
    render(<RunFilterBar filters={{}} onChange={onChange} />);
    const box = screen.getByLabelText('Target') as HTMLInputElement;
    fireEvent.change(box, { target: { value: 'a\u0007b' } });
    fireEvent.blur(box);
    expect(onChange).not.toHaveBeenCalled();
    expect(box.getAttribute('aria-invalid')).toBe('true');
    const alert = screen.getByRole('alert');
    expect(alert.textContent).toContain(`${MAX_TARGET_BYTES} bytes`);
    expect(box.getAttribute('aria-describedby')).toContain(alert.id);
    fireEvent.change(box, { target: { value: 'ok' } });
    fireEvent.blur(box);
    expect(onChange).toHaveBeenLastCalledWith({ target: 'ok' });
    expect(screen.queryByRole('alert')).toBeNull();
    expect(box.getAttribute('aria-invalid')).toBe('false');
  });

  it('shows Clear filters only while a filter is set, and clears them all', () => {
    const onChange = vi.fn();
    const { rerender } = render(<RunFilterBar filters={{}} onChange={onChange} />);
    expect(screen.queryByRole('button', { name: 'Clear filters' })).toBeNull();
    rerender(<RunFilterBar filters={{ state: 'failed', target: 'x', mode: 'sweep' }} onChange={onChange} />);
    fireEvent.click(screen.getByRole('button', { name: 'Clear filters' }));
    expect(onChange).toHaveBeenLastCalledWith({});
  });

  it('shows the typed target again as the new value when the filters change from outside', () => {
    const { rerender } = render(<RunFilterBar filters={{ target: 'a' }} onChange={() => undefined} />);
    expect((screen.getByLabelText('Target') as HTMLInputElement).value).toBe('a');
    rerender(<RunFilterBar filters={{}} onChange={() => undefined} />);
    expect((screen.getByLabelText('Target') as HTMLInputElement).value).toBe('');
  });

  it('does not submit the page when Enter is pressed', () => {
    render(<RunFilterBar filters={{}} onChange={() => undefined} />);
    const form = screen.getByRole('search');
    const submitted = fireEvent.submit(form);
    expect(submitted).toBe(false);
  });
});

describe('FailureFilterBar', () => {
  it('is a labelled search region with a labelled control for each filter', () => {
    render(<FailureFilterBar filters={{}} onChange={() => undefined} />);
    expect(screen.getByRole('search', { name: 'Filter failures' })).toBeTruthy();
    for (const name of ['Run id', 'Property id prefix', 'State']) expect(screen.getByLabelText(name)).toBeTruthy();
  });

  it('offers the six states the server derives, in the confirmation words', () => {
    render(<FailureFilterBar filters={{}} onChange={() => undefined} />);
    const options = Array.from((screen.getByLabelText('State') as HTMLSelectElement).options).map((o) => [o.value, o.textContent]);
    expect(options).toEqual([
      ['', 'All'],
      ['candidate', 'Candidate'],
      ['rerun_repeated', 'Rerun repeated'],
      ['confirmed', 'Confirmed'],
      ['verified', 'Verified'],
      ['not_reproducible', 'Not reproducible'],
      ['control_failed', 'Control failed'],
    ]);
  });

  it('applies a state when chosen, and explains that it reads each failure’s checks', () => {
    const onChange = vi.fn();
    const { rerender } = render(<FailureFilterBar filters={{}} onChange={onChange} />);
    expect(document.body.textContent).not.toContain('stored checks');
    fireEvent.change(screen.getByLabelText('State'), { target: { value: 'confirmed' } });
    expect(onChange).toHaveBeenLastCalledWith({ status: 'confirmed' });
    rerender(<FailureFilterBar filters={{ status: 'confirmed' }} onChange={onChange} />);
    expect(document.body.textContent).toContain('stored checks');
  });

  it('applies a run id and a property prefix on Enter, and keeps the other filters', () => {
    const onChange = vi.fn();
    render(<FailureFilterBar filters={{ status: 'candidate' }} onChange={onChange} />);
    const run = screen.getByLabelText('Run id');
    fireEvent.change(run, { target: { value: RUN } });
    fireEvent.keyDown(run, { key: 'Enter' });
    expect(onChange).toHaveBeenLastCalledWith({ status: 'candidate', run: RUN });
    const property = screen.getByLabelText('Property id prefix');
    fireEvent.change(property, { target: { value: 'db.' } });
    fireEvent.keyDown(property, { key: 'Enter' });
    expect(onChange).toHaveBeenLastCalledWith({ status: 'candidate', property: 'db.' });
  });

  it('describes what the property prefix matches', () => {
    render(<FailureFilterBar filters={{}} onChange={() => undefined} />);
    const box = screen.getByLabelText('Property id prefix');
    const hint = document.getElementById(box.getAttribute('aria-describedby') ?? '');
    expect(hint?.textContent).toContain('property whose id starts with this text');
  });

  it('refuses a run id that is not one, and a prefix over the byte limit, without calling back', () => {
    const onChange = vi.fn();
    render(<FailureFilterBar filters={{}} onChange={onChange} />);
    const run = screen.getByLabelText('Run id');
    fireEvent.change(run, { target: { value: 'run_12' } });
    fireEvent.blur(run);
    expect(screen.getByRole('alert').textContent).toContain('run_ followed by 16');
    const property = screen.getByLabelText('Property id prefix');
    fireEvent.change(property, { target: { value: 'é'.repeat(65) } });
    fireEvent.blur(property);
    expect(screen.getAllByRole('alert')).toHaveLength(2);
    expect(onChange).not.toHaveBeenCalled();
  });

  it('shows Clear filters only while a filter is set', () => {
    const onChange = vi.fn();
    const { rerender } = render(<FailureFilterBar filters={{}} onChange={onChange} />);
    expect(screen.queryByRole('button', { name: 'Clear filters' })).toBeNull();
    rerender(<FailureFilterBar filters={{ run: RUN }} onChange={onChange} />);
    fireEvent.click(screen.getByRole('button', { name: 'Clear filters' }));
    expect(onChange).toHaveBeenLastCalledWith({});
  });
});
