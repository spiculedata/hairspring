// @vitest-environment happy-dom
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import type { TailState } from './OutputTail';
import { OutputViewer } from './OutputViewer';
import type { StreamName } from './types';

afterEach(cleanup);

const tails: Partial<Record<StreamName, TailState>> = {
  stdout: { status: 'ready', text: 'start\nFound it\nmiddle\nfound again\nend\n' },
  stderr: { status: 'ready', text: 'warning one\n' },
};

function mount() {
  return render(<OutputViewer streams={['stdout', 'stderr']} tails={tails} omittedBytes={{ stdout: 0, stderr: 0 }} onDownload={() => undefined} />);
}

const find = (value: string) => fireEvent.change(screen.getByLabelText('Find in output'), { target: { value } });

describe('OutputViewer', () => {
  it('shows the first stream and switches to the other', () => {
    mount();
    expect(screen.getByRole('tab', { name: 'stdout' }).getAttribute('aria-selected')).toBe('true');
    expect(screen.getByRole('tabpanel').textContent).toContain('Found it');
    fireEvent.click(screen.getByRole('tab', { name: 'stderr' }));
    expect(screen.getByRole('tab', { name: 'stderr' }).getAttribute('aria-selected')).toBe('true');
    expect(screen.getByRole('tabpanel').textContent).toContain('warning one');
    expect(screen.getByRole('tabpanel').textContent).not.toContain('Found it');
  });

  it('counts matching lines, ignoring case, and marks them', () => {
    const { container } = mount();
    find('found');
    expect(screen.getByText('2 matching lines')).toBeTruthy();
    expect(container.querySelectorAll('.out-line.out-hit').length).toBe(2);
  });

  it('can show only the matching lines, keeping their own line numbers', () => {
    const { container } = mount();
    find('found');
    fireEvent.click(screen.getByLabelText('Only matching lines'));
    const numbers = [...container.querySelectorAll('.out-line .ln')].map((n) => n.textContent);
    expect(numbers).toEqual(['2', '4']);
  });

  it('says when nothing matches', () => {
    mount();
    find('zzz');
    expect(screen.getByText('No matching lines')).toBeTruthy();
  });

  it('names the focusable output region after its stream', () => {
    const { container } = mount();
    const pre = container.querySelector('pre.output-tail');
    expect(pre?.getAttribute('aria-label')).toBe('stdout tail');
    expect(pre?.getAttribute('tabindex')).toBe('0');
  });

  it('keeps only the selected tab in the tab order', () => {
    mount();
    expect(screen.getByRole('tab', { name: 'stdout' }).getAttribute('tabindex')).toBe('0');
    expect(screen.getByRole('tab', { name: 'stderr' }).getAttribute('tabindex')).toBe('-1');
  });

  it('moves between tabs with the arrow, Home and End keys, selecting and focusing the target', () => {
    mount();
    const stdout = screen.getByRole('tab', { name: 'stdout' });
    const stderr = screen.getByRole('tab', { name: 'stderr' });
    fireEvent.keyDown(stdout, { key: 'ArrowRight' });
    expect(stderr.getAttribute('aria-selected')).toBe('true');
    expect(document.activeElement).toBe(stderr);
    fireEvent.keyDown(stderr, { key: 'ArrowRight' });
    expect(stdout.getAttribute('aria-selected')).toBe('true');
    fireEvent.keyDown(stdout, { key: 'ArrowLeft' });
    expect(stderr.getAttribute('aria-selected')).toBe('true');
    fireEvent.keyDown(stderr, { key: 'Home' });
    expect(stdout.getAttribute('aria-selected')).toBe('true');
    fireEvent.keyDown(stdout, { key: 'End' });
    expect(stderr.getAttribute('aria-selected')).toBe('true');
    expect(document.activeElement).toBe(stderr);
  });

  it('does not hide a panel with both the hidden attribute and an inline style', () => {
    mount();
    const hidden = document.querySelector('[role="tabpanel"][hidden]');
    expect(hidden).not.toBeNull();
    expect(hidden?.getAttribute('style')).toBeNull();
  });
});
