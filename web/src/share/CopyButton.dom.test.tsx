// @vitest-environment happy-dom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { CopyButton, CopyButtons } from './CopyButton';

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  Reflect.deleteProperty(navigator, 'clipboard');
});

function stubClipboard(writeText: (t: string) => Promise<void>) {
  Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
}

const click = async () => {
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: 'Copy link' }));
  });
};

describe('CopyButton', () => {
  it('writes the text at the time of the click and says it was copied', async () => {
    const writeText = vi.fn(async () => undefined);
    stubClipboard(writeText);
    let current = 'first';
    render(<CopyButton label="Copy link" text={() => current} />);
    current = 'second';
    await click();
    expect(writeText).toHaveBeenCalledWith('second');
    expect(screen.getByRole('status').textContent).toBe('Copied');
  });

  it('says it could not copy when the browser refuses, instead of claiming success', async () => {
    stubClipboard(async () => {
      throw new Error('denied');
    });
    render(<CopyButton label="Copy link" text={() => 'x'} />);
    await click();
    expect(screen.getByRole('status').textContent).toContain('Could not copy');
  });

  it('says it could not copy when there is no clipboard', async () => {
    Object.defineProperty(navigator, 'clipboard', { value: undefined, configurable: true });
    render(<CopyButton label="Copy link" text={() => 'x'} />);
    await click();
    expect(screen.getByRole('status').textContent).toContain('Could not copy');
  });
});

describe('CopyButton when the text cannot be read', () => {
  it('says it could not copy when text() throws, instead of leaving the press without feedback', async () => {
    stubClipboard(async () => undefined);
    render(
      <CopyButton
        label="Copy link"
        text={() => {
          throw new Error('no text');
        }}
      />,
    );
    await click();
    expect(screen.getByRole('status').textContent).toContain('Could not copy');
  });
});

describe('CopyButtons', () => {
  const items = [
    { label: 'Copy link', text: () => 'a' },
    { label: 'Copy summary', text: () => 'b' },
    { label: 'Copy bundle', text: () => 'c' },
  ];
  const press = async (name: string) => {
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name }));
    });
  };

  it('keeps one live region for the whole group, so a second copy replaces the first message', async () => {
    stubClipboard(async () => undefined);
    render(<CopyButtons items={items} />);
    for (const { label } of items) await press(label);
    const regions = screen.getAllByRole('status');
    expect(regions).toHaveLength(1);
    expect(regions[0]?.textContent).toBe('Copied');
  });

  it('shows the failure message for the last button pressed, not a stale success', async () => {
    let fail = false;
    stubClipboard(async () => {
      if (fail) throw new Error('denied');
    });
    render(<CopyButtons items={items} />);
    await press('Copy link');
    fail = true;
    await press('Copy bundle');
    expect(screen.getByRole('status').textContent).toContain('Could not copy');
  });
});
