// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest';
import { THEME_KEY, applyTheme, parseTheme, readStoredTheme, storeTheme, type ThemeStore } from './theme';

const memory = (initial?: string): ThemeStore & { value: string | undefined } => {
  const box = { value: initial } as ThemeStore & { value: string | undefined };
  box.getItem = () => box.value ?? null;
  box.setItem = (_k, v) => {
    box.value = v;
  };
  return box;
};
const broken: ThemeStore = {
  getItem: () => {
    throw new Error('blocked');
  },
  setItem: () => {
    throw new Error('blocked');
  },
};

describe('theme mode', () => {
  it('accepts only system, light and dark, and treats anything else as system', () => {
    expect(parseTheme('light')).toBe('light');
    expect(parseTheme('dark')).toBe('dark');
    expect(parseTheme('system')).toBe('system');
    for (const bad of [null, undefined, '', 'Dark', 'blue', '{}']) expect(parseTheme(bad)).toBe('system');
  });

  it('reads and stores the choice under one key', () => {
    const store = memory();
    expect(readStoredTheme(store)).toBe('system');
    expect(storeTheme('dark', store)).toBe(true);
    expect(store.value).toBe('dark');
    expect(readStoredTheme(store)).toBe('dark');
    expect(THEME_KEY).toMatch(/^hairspring/);
  });

  it('falls back to system and reports a failed save when storage throws or is missing', () => {
    expect(readStoredTheme(broken)).toBe('system');
    expect(storeTheme('light', broken)).toBe(false);
    expect(readStoredTheme(null)).toBe('system');
    expect(storeTheme('light', null)).toBe(false);
    expect(readStoredTheme(memory('nonsense'))).toBe('system');
  });

  it('sets data-theme for light and dark and removes it for system', () => {
    const root = document.createElement('html');
    applyTheme('dark', root);
    expect(root.getAttribute('data-theme')).toBe('dark');
    applyTheme('light', root);
    expect(root.getAttribute('data-theme')).toBe('light');
    applyTheme('system', root);
    expect(root.hasAttribute('data-theme')).toBe(false);
  });
});
