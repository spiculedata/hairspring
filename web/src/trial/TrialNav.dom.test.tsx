// @vitest-environment happy-dom
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { TrialNav } from './TrialNav';

afterEach(cleanup);

const items = [
  { id: 'schedule-title', label: 'Schedule' },
  { id: 'output-title', label: 'Output' },
];

function mountWithHeadings() {
  render(
    <>
      <TrialNav items={items} />
      <h2 id="schedule-title" tabIndex={-1}>Schedule</h2>
      <h2 id="output-title" tabIndex={-1}>Output</h2>
    </>,
  );
  const target = document.getElementById('output-title') as HTMLElement;
  target.scrollIntoView = vi.fn();
  return target;
}

describe('TrialNav', () => {
  it('scrolls to the section and moves focus to its heading when a button is pressed', () => {
    const target = mountWithHeadings();
    fireEvent.click(screen.getByRole('button', { name: 'Output' }));
    expect(target.scrollIntoView).toHaveBeenCalled();
    expect(document.activeElement).toBe(target);
  });

  it('moves focus to the heading when the select is used', () => {
    const target = mountWithHeadings();
    fireEvent.change(screen.getByLabelText('Jump to section'), { target: { value: 'output-title' } });
    expect(document.activeElement).toBe(target);
  });
});
