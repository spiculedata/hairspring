// @vitest-environment happy-dom
import { act, cleanup, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ScrollRegion } from './ScrollRegion';

/** The box sizes the fake layout reports: happy-dom does no layout, so a test says how wide things are. */
const sizes = { scrollWidth: 968, clientWidth: 966 };

class FakeResizeObserver {
  static instances: FakeResizeObserver[] = [];
  observed = new Set<Element>();
  disconnected = false;
  constructor(private callback: ResizeObserverCallback) {
    FakeResizeObserver.instances.push(this);
  }
  observe(el: Element) {
    this.observed.add(el);
  }
  unobserve(el: Element) {
    this.observed.delete(el);
  }
  disconnect() {
    this.observed.clear();
    this.disconnected = true;
  }
  fire() {
    this.callback([], this as unknown as ResizeObserver);
  }
}

const hint = () => screen.getByText('Scroll sideways.');
const region = () => screen.getByRole('region', { name: 'Judged trials' });

function Region({ tableKey = 'a' }: { tableKey?: string }) {
  return (
    <ScrollRegion label="Judged trials" hint="Scroll sideways." maxHeight={400}>
      <table key={tableKey}>
        <tbody>
          <tr>
            <td>cell</td>
          </tr>
        </tbody>
      </table>
    </ScrollRegion>
  );
}

describe('ScrollRegion in a DOM', () => {
  beforeEach(() => {
    sizes.scrollWidth = 968;
    sizes.clientWidth = 966;
    FakeResizeObserver.instances = [];
    vi.stubGlobal('ResizeObserver', FakeResizeObserver);
    Object.defineProperty(HTMLElement.prototype, 'scrollWidth', { configurable: true, get: () => sizes.scrollWidth });
    Object.defineProperty(HTMLElement.prototype, 'clientWidth', { configurable: true, get: () => sizes.clientWidth });
  });
  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    delete (HTMLElement.prototype as unknown as Record<string, unknown>).scrollWidth;
    delete (HTMLElement.prototype as unknown as Record<string, unknown>).clientWidth;
  });

  it('shows the hint and marks the box while the table is wider than it, even by a few pixels', () => {
    sizes.scrollWidth = 968;
    sizes.clientWidth = 961;
    render(<Region />);
    expect(hint().hasAttribute('hidden')).toBe(false);
    expect(region().getAttribute('data-overflowing')).toBe('true');
  });

  it('hides the hint when the table fits, ignoring a pixel of rounding', () => {
    sizes.scrollWidth = 967;
    sizes.clientWidth = 966;
    render(<Region />);
    expect(hint().hasAttribute('hidden')).toBe(true);
    expect(region().getAttribute('data-overflowing')).toBe('false');
  });

  it('measures again when the box is resized', () => {
    sizes.scrollWidth = 944;
    sizes.clientWidth = 966;
    render(<Region />);
    expect(hint().hasAttribute('hidden')).toBe(true);
    sizes.clientWidth = 940;
    act(() => FakeResizeObserver.instances[0].fire());
    expect(hint().hasAttribute('hidden')).toBe(false);
    sizes.clientWidth = 966;
    act(() => FakeResizeObserver.instances[0].fire());
    expect(hint().hasAttribute('hidden')).toBe(true);
  });

  it('observes the table it holds now, not only the one it first held', () => {
    const { rerender } = render(<Region tableKey="a" />);
    const observer = FakeResizeObserver.instances[0];
    const first = region().firstElementChild;
    expect(observer.observed.has(region())).toBe(true);
    expect(observer.observed.has(first as Element)).toBe(true);
    rerender(<Region tableKey="b" />);
    const second = region().firstElementChild;
    expect(second).not.toBe(first);
    expect(observer.observed.has(second as Element)).toBe(true);
    expect(observer.observed.has(first as Element)).toBe(false);
  });

  it('stops observing when it goes away', () => {
    const { unmount } = render(<Region />);
    const observer = FakeResizeObserver.instances[0];
    unmount();
    expect(observer.disconnected).toBe(true);
  });

  it('still measures once where there is no ResizeObserver', () => {
    vi.stubGlobal('ResizeObserver', undefined);
    sizes.scrollWidth = 968;
    sizes.clientWidth = 966;
    render(<Region />);
    expect(hint().hasAttribute('hidden')).toBe(false);
  });
});
