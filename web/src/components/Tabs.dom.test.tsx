// @vitest-environment happy-dom
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { useState } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { Tabs, type TabSpec } from './Tabs';

const TABS: readonly TabSpec[] = [
  { id: 'one', label: 'One' },
  { id: 'two', label: 'Two', count: 3 },
  { id: 'three', label: 'Three' },
];

function Harness({ onSelect }: { onSelect?: (id: string) => void }) {
  const [selected, setSelected] = useState('one');
  return (
    <Tabs
      label="Sections"
      idPrefix="t"
      tabs={TABS}
      selected={selected}
      onSelect={(id) => {
        onSelect?.(id);
        setSelected(id);
      }}
    >
      <p>panel for {selected}</p>
    </Tabs>
  );
}

const tab = (name: RegExp) => screen.getByRole('tab', { name });

describe('Tabs', () => {
  afterEach(cleanup);

  it('exposes a tablist with one selected tab and a labelled panel', () => {
    render(<Harness />);
    expect(screen.getByRole('tablist', { name: 'Sections' })).toBeTruthy();
    expect(screen.getAllByRole('tab')).toHaveLength(3);
    expect(tab(/^One/).getAttribute('aria-selected')).toBe('true');
    expect(tab(/^Two/).getAttribute('aria-selected')).toBe('false');
    const panel = screen.getByRole('tabpanel');
    expect(panel.getAttribute('aria-labelledby')).toBe(tab(/^One/).id);
    expect(tab(/^One/).getAttribute('aria-controls')).toBe(panel.id);
    expect(panel.textContent).toContain('panel for one');
  });

  it('puts only the selected tab in the tab order', () => {
    render(<Harness />);
    expect(tab(/^One/).tabIndex).toBe(0);
    expect(tab(/^Two/).tabIndex).toBe(-1);
    expect(tab(/^Three/).tabIndex).toBe(-1);
  });

  it('shows a count in the tab name', () => {
    render(<Harness />);
    expect(screen.getByRole('tab', { name: 'Two 3' })).toBeTruthy();
  });

  it('selects on click', () => {
    const onSelect = vi.fn();
    render(<Harness onSelect={onSelect} />);
    fireEvent.click(tab(/^Three/));
    expect(onSelect).toHaveBeenCalledWith('three');
    expect(screen.getByRole('tabpanel').textContent).toContain('panel for three');
  });

  it('moves selection and focus with the arrow keys, wrapping at both ends', () => {
    render(<Harness />);
    fireEvent.keyDown(tab(/^One/), { key: 'ArrowRight' });
    expect(tab(/^Two/).getAttribute('aria-selected')).toBe('true');
    expect(document.activeElement).toBe(tab(/^Two/));
    expect(tab(/^Two/).tabIndex).toBe(0);
    expect(tab(/^One/).tabIndex).toBe(-1);
    fireEvent.keyDown(tab(/^Two/), { key: 'ArrowLeft' });
    fireEvent.keyDown(tab(/^One/), { key: 'ArrowLeft' });
    expect(document.activeElement).toBe(tab(/^Three/));
    fireEvent.keyDown(tab(/^Three/), { key: 'ArrowRight' });
    expect(document.activeElement).toBe(tab(/^One/));
  });

  it('goes to the first and last tab with Home and End', () => {
    render(<Harness />);
    fireEvent.keyDown(tab(/^One/), { key: 'End' });
    expect(document.activeElement).toBe(tab(/^Three/));
    expect(screen.getByRole('tabpanel').textContent).toContain('panel for three');
    fireEvent.keyDown(tab(/^Three/), { key: 'Home' });
    expect(document.activeElement).toBe(tab(/^One/));
  });

  it('ignores other keys', () => {
    const onSelect = vi.fn();
    render(<Harness onSelect={onSelect} />);
    fireEvent.keyDown(tab(/^One/), { key: 'a' });
    fireEvent.keyDown(tab(/^One/), { key: 'ArrowDown' });
    expect(onSelect).not.toHaveBeenCalled();
  });

  it('makes the panel reachable by keyboard', () => {
    render(<Harness />);
    expect(screen.getByRole('tabpanel').tabIndex).toBe(0);
  });
});
