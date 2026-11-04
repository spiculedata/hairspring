/** The colour theme the reader chose: follow the system, or force light or dark. Kept per browser. */
export type ThemeMode = 'system' | 'light' | 'dark';

export const THEME_MODES: readonly ThemeMode[] = ['system', 'light', 'dark'];
export const THEME_KEY = 'hairspring-theme';

/** The part of `localStorage` used here, so tests can pass a stand-in. */
export interface ThemeStore {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

export function parseTheme(value: string | null | undefined): ThemeMode {
  return value === 'light' || value === 'dark' ? value : 'system';
}

/** The browser's storage, or undefined where it is blocked or missing (some private windows throw on access). */
export function browserStore(): ThemeStore | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

/** The stored choice; system when nothing usable is stored or storage cannot be read. */
export function readStoredTheme(store: ThemeStore | null = browserStore()): ThemeMode {
  try {
    return parseTheme(store?.getItem(THEME_KEY));
  } catch {
    return 'system';
  }
}

/** Remembers the choice. False when storage refused, in which case the choice lasts until the page is closed. */
export function storeTheme(mode: ThemeMode, store: ThemeStore | null = browserStore()): boolean {
  if (!store) return false;
  try {
    store.setItem(THEME_KEY, mode);
    return true;
  } catch {
    return false;
  }
}

/** Light and dark are set on the root element for the stylesheet to read; system removes the setting. */
export function applyTheme(mode: ThemeMode, root: HTMLElement = document.documentElement): void {
  if (mode === 'system') root.removeAttribute('data-theme');
  else root.setAttribute('data-theme', mode);
}
