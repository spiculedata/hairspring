// @vitest-environment happy-dom
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { LoadMore } from './LoadMore';

afterEach(cleanup);

const base = {
  label: 'Load more runs (50 shown)',
  what: 'runs' as const,
  shown: 50,
  hasMore: true,
  loading: false,
  onLoadMore: () => undefined,
  error: undefined,
  moreLoads: 0,
};

describe('LoadMore', () => {
  it('is a real button with the label that says how many are shown, and it asks for more when pressed', () => {
    const onLoadMore = vi.fn();
    render(<LoadMore {...base} onLoadMore={onLoadMore} />);
    const button = screen.getByRole('button', { name: 'Load more runs (50 shown)' });
    expect(button.getAttribute('type')).toBe('button');
    fireEvent.click(button);
    expect(onLoadMore).toHaveBeenCalledTimes(1);
  });

  it('draws no button when there is nothing older', () => {
    render(<LoadMore {...base} hasMore={false} />);
    expect(screen.queryByRole('button')).toBeNull();
  });

  it('disables the button and says it is loading while a request is out', () => {
    render(<LoadMore {...base} loading />);
    const button = screen.getByRole('button') as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    expect(button.getAttribute('aria-busy')).toBe('true');
    expect(button.textContent).toBe('Loading more…');
  });

  it('keeps the button and says what failed, as an alert, so asking again tries again', () => {
    render(<LoadMore {...base} error="database is locked" />);
    expect(screen.getByRole('alert').textContent).toBe('Could not load more runs: database is locked');
    expect((screen.getByRole('button') as HTMLButtonElement).disabled).toBe(false);
  });

  it('shows the reason as escaped text, never as markup', () => {
    render(<LoadMore {...base} error="<img src=x onerror=1>" />);
    expect(document.querySelector('img')).toBeNull();
  });

  it('says nothing in its live note until the reader has asked for more, then how many are shown', () => {
    const { rerender } = render(<LoadMore {...base} />);
    expect(screen.getByRole('status').textContent).toBe('');
    rerender(<LoadMore {...base} moreLoads={1} shown={100} what="failures" />);
    expect(screen.getByRole('status').textContent).toBe('100 failures shown.');
  });

  it('keeps one live region in the page, so a screen reader registers it before it speaks', () => {
    const { rerender } = render(<LoadMore {...base} />);
    const region = screen.getByRole('status');
    rerender(<LoadMore {...base} moreLoads={2} />);
    expect(screen.getByRole('status')).toBe(region);
  });
});
