// @vitest-environment happy-dom
import { cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { THEME_KEY } from './theme';
import { ThemeSwitch } from './ThemeSwitch';

describe('ThemeSwitch', () => {
  beforeEach(() => {
    localStorage.clear();
    document.documentElement.removeAttribute('data-theme');
  });
  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
  });

  it('is a labelled group of three buttons, system pressed by default', () => {
    const { getByRole, getAllByRole } = render(<ThemeSwitch />);
    expect(getByRole('group', { name: 'Colour theme' })).toBeTruthy();
    const buttons = getAllByRole('button');
    expect(buttons.map((b) => b.textContent)).toEqual(['System', 'Light', 'Dark']);
    expect(buttons.map((b) => b.getAttribute('aria-pressed'))).toEqual(['true', 'false', 'false']);
    expect(buttons.every((b) => b.getAttribute('type') === 'button')).toBe(true);
  });

  it('chooses a theme, sets data-theme and remembers it', () => {
    const { getByRole } = render(<ThemeSwitch />);
    fireEvent.click(getByRole('button', { name: 'Dark' }));
    expect(document.documentElement.getAttribute('data-theme')).toBe('dark');
    expect(localStorage.getItem(THEME_KEY)).toBe('dark');
    expect(getByRole('button', { name: 'Dark' }).getAttribute('aria-pressed')).toBe('true');
    expect(getByRole('button', { name: 'System' }).getAttribute('aria-pressed')).toBe('false');
    fireEvent.click(getByRole('button', { name: 'System' }));
    expect(document.documentElement.hasAttribute('data-theme')).toBe(false);
  });

  it('starts from the stored choice', () => {
    localStorage.setItem(THEME_KEY, 'light');
    const { getByRole } = render(<ThemeSwitch />);
    expect(getByRole('button', { name: 'Light' }).getAttribute('aria-pressed')).toBe('true');
    expect(document.documentElement.getAttribute('data-theme')).toBe('light');
  });

  it('still switches for this page when storage is blocked', () => {
    vi.stubGlobal('localStorage', {
      getItem: () => {
        throw new Error('blocked');
      },
      setItem: () => {
        throw new Error('blocked');
      },
    });
    const { getByRole } = render(<ThemeSwitch />);
    fireEvent.click(getByRole('button', { name: 'Dark' }));
    expect(document.documentElement.getAttribute('data-theme')).toBe('dark');
    expect(getByRole('button', { name: 'Dark' }).getAttribute('aria-pressed')).toBe('true');
  });
});
