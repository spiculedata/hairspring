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
  endReached: false,
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

  it('marks the button busy and says it is loading while a request is out, without taking it out of the tab order', () => {
    const onLoadMore = vi.fn();
    render(<LoadMore {...base} loading onLoadMore={onLoadMore} />);
    const button = screen.getByRole('button') as HTMLButtonElement;
    expect(button.disabled).toBe(false);
    expect(button.getAttribute('aria-disabled')).toBe('true');
    expect(button.getAttribute('aria-busy')).toBe('true');
    expect(button.textContent).toBe('Loading more…');
    fireEvent.click(button);
    expect(onLoadMore).not.toHaveBeenCalled();
  });

  it('keeps keyboard focus on the button while a request is out', () => {
    const { rerender } = render(<LoadMore {...base} />);
    const button = screen.getByRole('button');
    button.focus();
    rerender(<LoadMore {...base} loading />);
    expect(document.activeElement).toBe(screen.getByRole('button'));
    rerender(<LoadMore {...base} moreLoads={1} />);
    expect(document.activeElement).toBe(screen.getByRole('button'));
  });

  it('moves focus to the end-of-list note when the last page arrives and the button had focus', () => {
    const { rerender } = render(<LoadMore {...base} />);
    screen.getByRole('button').focus();
    rerender(<LoadMore {...base} hasMore={false} moreLoads={1} shown={80} />);
    expect(screen.queryByRole('button')).toBeNull();
    const note = screen.getByText('All 80 runs are shown.');
    expect(document.activeElement).toBe(note);
  });

  it('leaves focus alone when the last page arrives and the button did not have it', () => {
    const { rerender } = render(<LoadMore {...base} />);
    rerender(<LoadMore {...base} hasMore={false} moreLoads={1} shown={80} />);
    expect(document.activeElement).toBe(document.body);
  });

  it('says the list cannot be read further, as a note and not as a failure, when the server stopped it', () => {
    render(<LoadMore {...base} hasMore={false} endReached moreLoads={3} shown={500} />);
    expect(screen.queryByRole('alert')).toBeNull();
    expect(screen.getByText(/as far back as this list can be read/i)).toBeTruthy();
  });

  it('says nothing about the end before the reader has asked for more', () => {
    render(<LoadMore {...base} hasMore={false} />);
    expect(screen.queryByText(/are shown/)).toBeNull();
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
