import { describe, expect, it } from 'vitest';
import { readSource } from '../readSource';
import { THEME_KEY, THEME_MODES } from './theme';

const html = readSource('../index.html');
const init = readSource('../public/theme-init.js');
const css = readSource('shell/shell.css');

/** Runs the head script against a stand-in page and storage, and returns what it set on the root element. */
function runInit(storage: unknown): string | null {
  let theme: string | null = null;
  const document = { documentElement: { setAttribute: (_n: string, v: string) => (theme = v) } };
  new Function('localStorage', 'document', init)(storage, document);
  return theme;
}

describe('theme before first paint', () => {
  it('is a same-origin classic script in the head, ahead of the app, with no inline script (the server sends a CSP of default-src self)', () => {
    const head = html.slice(html.indexOf('<head>'), html.indexOf('</head>'));
    expect(head).toMatch(/<script src="\/theme-init\.js"><\/script>/);
    expect(head).not.toMatch(/<script(?![^>]*\bsrc=)[^>]*>/);
    expect(html.indexOf('/theme-init.js')).toBeLessThan(html.indexOf('/src/main.tsx'));
  });

  it('reads the same key and accepts the same values as theme.ts', () => {
    expect(init).toContain(`'${THEME_KEY}'`);
    for (const mode of THEME_MODES.filter((m) => m !== 'system')) expect(init).toContain(`'${mode}'`);
    expect(init).toContain('data-theme');
  });

  it('sets light and dark, ignores system and anything else, and survives blocked storage', () => {
    const store = (v: string | null) => ({ getItem: (k: string) => (k === THEME_KEY ? v : 'wrong-key') });
    expect(runInit(store('dark'))).toBe('dark');
    expect(runInit(store('light'))).toBe('light');
    for (const v of ['system', 'Dark', '', null]) expect(runInit(store(v))).toBeNull();
    const blocked = {
      getItem: () => {
        throw new Error('blocked');
      },
    };
    expect(() => runInit(blocked)).not.toThrow();
    expect(runInit(blocked)).toBeNull();
    expect(() => new Function('localStorage', 'document', init)(undefined, {})).not.toThrow();
  });
});

describe('shell sizing', () => {
  const rule = (selector: string): string => {
    const start = css.indexOf(`${selector} {`);
    expect(start, `${selector} rule`).toBeGreaterThanOrEqual(0);
    return css.slice(start, css.indexOf('}', start));
  };

  it('keeps theme buttons at least 2rem tall for touch targets', () => {
    const m = /min-height:\s*([\d.]+)rem/.exec(rule('.theme-switch button'));
    expect(m && Number(m[1])).toBeGreaterThanOrEqual(2);
  });

  it('does not force the side navigation to a viewport-based height', () => {
    expect(rule('.sidenav')).not.toMatch(/vh/);
  });
});
