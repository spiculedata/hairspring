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
});
