// @vitest-environment happy-dom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { CopyButton } from './CopyButton';

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
