/// <reference types="node" />
import { readdirSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { contrastRatio, parseHex } from './contrast';
import { readSource } from './readSource';

const TEXT_MIN = 4.5; // WCAG 1.4.3
const EDGE_MIN = 3; // WCAG 1.4.11

const css = readSource('styles.css');

/** The declarations of the `:root` block that is directly inside `scope` (the whole file when it is empty). */
function tokens(scope: 'light' | 'dark'): Readonly<Record<string, string>> {
  const source =
    scope === 'light'
      ? css.slice(0, css.indexOf('@media (prefers-color-scheme: dark)'))
      : css.slice(css.indexOf('@media (prefers-color-scheme: dark)'));
  const block = /:root\s*\{([^}]*)\}/.exec(source);
  if (!block) throw new Error(`no :root block for ${scope}`);
  const out: Record<string, string> = {};
  for (const m of block[1].matchAll(/(--[a-z0-9-]+)\s*:\s*([^;]+);/g)) out[m[1]] = m[2].trim();
  return out;
}

/** Light values, with the dark block laid over them, as the browser computes them in dark mode. */
const light = tokens('light');
const THEMES = { light, dark: { ...light, ...tokens('dark') } } as const;

const STATUS = ['fail', 'gap', 'pass', 'live', 'neutral', 'confirm'] as const;
const GROUNDS = ['--bg', '--surface', '--surface-2'] as const;

function ratio(theme: Readonly<Record<string, string>>, fg: string, bg: string): number {
  const f = theme[fg];
  const b = theme[bg];
  if (f === undefined || b === undefined) throw new Error(`missing token ${f === undefined ? fg : bg}`);
  return contrastRatio(f, b);
}

describe('contrastRatio', () => {
  it('matches the WCAG reference values', () => {
    expect(contrastRatio('#000000', '#ffffff')).toBeCloseTo(21, 5);
    expect(contrastRatio('#ffffff', '#ffffff')).toBeCloseTo(1, 5);
    // The values the old stylesheet had, which the review measured: white on dark-mode amber failed.
    expect(contrastRatio('#ffffff', '#f59e0b')).toBeCloseTo(2.15, 1);
    expect(contrastRatio('#ffffff', '#b45309')).toBeCloseTo(5.02, 1);
  });
  it('is symmetric and reads three-digit hex', () => {
    expect(contrastRatio('#fff', '#000')).toBeCloseTo(21, 5);
    expect(contrastRatio('#1c1917', '#fafaf9')).toBeCloseTo(contrastRatio('#fafaf9', '#1c1917'), 10);
  });
  it('rejects a value that is not a hex colour', () => {
    expect(() => parseHex('rebeccapurple')).toThrow(/hex colour/);
  });
});

for (const [name, theme] of Object.entries(THEMES)) {
  describe(`design tokens, ${name} theme`, () => {
    it('defines every token the contrast checks rely on', () => {
      for (const t of ['--bg', '--surface', '--surface-2', '--text', '--text-muted', '--text-subtle', '--border', '--border-strong', '--accent', '--accent-ink', '--focus']) {
        expect(theme[t], t).toMatch(/^#[0-9a-f]{6}$/i);
      }
      for (const s of STATUS) {
        expect(theme[`--${s}`], s).toMatch(/^#[0-9a-f]{6}$/i);
        expect(theme[`--${s}-bg`], `${s}-bg`).toMatch(/^#[0-9a-f]{6}$/i);
      }
    });

    it(`gives body, muted and subtle text at least ${TEXT_MIN}:1 on every ground`, () => {
      for (const fg of ['--text', '--text-muted', '--text-subtle']) {
        for (const bg of GROUNDS) expect(ratio(theme, fg, bg), `${fg} on ${bg}`).toBeGreaterThanOrEqual(TEXT_MIN);
      }
    });

    it(`gives links and the accent at least ${TEXT_MIN}:1 as text on the page and card grounds`, () => {
      for (const bg of ['--bg', '--surface']) expect(ratio(theme, '--accent', bg), `accent on ${bg}`).toBeGreaterThanOrEqual(TEXT_MIN);
    });

    it(`gives the text on a primary button at least ${TEXT_MIN}:1 against the button`, () => {
      expect(ratio(theme, '--accent-ink', '--accent')).toBeGreaterThanOrEqual(TEXT_MIN);
    });

    it(`gives every status colour at least ${TEXT_MIN}:1 on its own tint and on the page and card grounds`, () => {
      for (const s of STATUS) {
        expect(ratio(theme, `--${s}`, `--${s}-bg`), `${s} on its tint`).toBeGreaterThanOrEqual(TEXT_MIN);
        expect(ratio(theme, `--${s}`, '--bg'), `${s} on bg`).toBeGreaterThanOrEqual(TEXT_MIN);
        expect(ratio(theme, `--${s}`, '--surface'), `${s} on surface`).toBeGreaterThanOrEqual(TEXT_MIN);
      }
    });

    it(`gives control edges and the focus ring at least ${EDGE_MIN}:1 against the page and card grounds`, () => {
      for (const bg of ['--bg', '--surface']) {
        expect(ratio(theme, '--border-strong', bg), `border-strong on ${bg}`).toBeGreaterThanOrEqual(EDGE_MIN);
        expect(ratio(theme, '--focus', bg), `focus on ${bg}`).toBeGreaterThanOrEqual(EDGE_MIN);
      }
    });

    it('keeps amber for caution only: the clickable accent is not amber and the caution colour is', () => {
      const hue = (hex: string): number => {
        const [r, g, b] = parseHex(hex).map((c) => c / 255);
        const max = Math.max(r, g, b);
        const d = max - Math.min(r, g, b);
        if (d === 0) return 0;
        const h = max === r ? ((g - b) / d) % 6 : max === g ? (b - r) / d + 2 : (r - g) / d + 4;
        return (h * 60 + 360) % 360;
      };
      const AMBER = [15, 55] as const;
      expect(hue(theme['--accent'])).toBeGreaterThan(AMBER[1] + 100);
      expect(hue(theme['--gap'])).toBeGreaterThanOrEqual(AMBER[0]);
      expect(hue(theme['--gap'])).toBeLessThanOrEqual(AMBER[1]);
    });
  });
}

describe('stylesheet use of the tokens', () => {
  const rules = css.slice(css.indexOf('* { box-sizing'));

  it('writes no colour literal outside the token blocks', () => {
    expect(rules).not.toMatch(/#[0-9a-fA-F]{3,8}\b|\brgba?\(|\bhsla?\(/);
  });

  it('paints a primary button with the accent and its own ink, not white', () => {
    expect(rules).toMatch(/button \{[^}]*background: var\(--accent\)[^}]*color: var\(--accent-ink\)/);
  });

  it('draws form control edges with the strong border token', () => {
    expect(rules).toMatch(/input, select[^{]*\{[^}]*border: 1px solid var\(--border-strong\)/);
  });

  it('has one focus ring, in the focus token', () => {
    expect(rules).toMatch(/:focus-visible \{[^}]*outline: 2px solid var\(--focus\)/);
  });

  it('no longer uses the old busy token for a state; running is the live colour and caution is the gap colour', () => {
    for (const file of ['styles.css', 'explorer/explorer.css', 'seeds/seeds.css', 'trial/trial.css', 'seeds/StripCanvas.tsx']) {
      expect(readSource(file), file).not.toContain('--busy');
    }
    expect(rules).toMatch(/\.badge-busy \{[^}]*var\(--live\)/);
  });

  it('gives every status badge a mark as well as a word and a colour, as generated content with no spoken text', () => {
    const marks = new Set<string>();
    for (const tone of ['good', 'bad', 'busy', 'idle']) {
      const m = new RegExp(`\\.badge-${tone}::before \\{[^}]*content: '([^']+)' / ''`).exec(rules);
      expect(m, `badge-${tone}`).not.toBeNull();
      marks.add(m![1]);
    }
    expect(marks.size).toBe(4);
  });
});

describe('every stylesheet and the canvas code use the tokens', () => {
  const dir = new URL('./', import.meta.url);
  const cssFiles = (readdirSync(dir, { recursive: true }) as string[]).filter((f) => f.endsWith('.css')).sort();
  const NAMED = 'red|white|black|blue|green|gray|grey|orange|yellow|purple|pink|brown|cyan|magenta|silver|gold|navy|teal|maroon|olive|lime|aqua|fuchsia|indigo|violet|crimson|salmon|tomato|coral|khaki|beige|ivory|tan|lightgray|lightgrey|darkgray|darkgrey|dimgray|dimgrey|slategray|whitesmoke|gainsboro';
  const NAMED_IN_CSS = new RegExp(`(?:color|background(?:-color)?|border(?:-[a-z]+)?|outline(?:-color)?|fill|stroke)\\s*:[^;{}]*\\b(?:${NAMED})\\b`, 'i');
  const NAMED_IN_CODE = new RegExp(`['"\`](?:${NAMED})['"\`]`, 'i');
  const LITERAL = /#[0-9a-fA-F]{3,8}\b|\brgba?\(|\bhsla?\(/;

  it('finds all four stylesheets', () => {
    expect(cssFiles).toEqual(['explorer/explorer.css', 'seeds/seeds.css', 'styles.css', 'trial/trial.css']);
  });

  it('writes no colour literal in any stylesheet outside the token blocks of styles.css', () => {
    for (const file of cssFiles) {
      const text = readSource(file);
      const body = file === 'styles.css' ? text.slice(text.indexOf('* { box-sizing')) : text;
      expect(body, file).not.toMatch(LITERAL);
      expect(body, file).not.toMatch(NAMED_IN_CSS);
    }
  });

  it('draws every focus ring in the focus token, not the accent', () => {
    for (const file of cssFiles) {
      for (const rule of readSource(file).matchAll(/:focus-visible[^{]*\{([^}]*)\}/g)) {
        expect(rule[1], file).toMatch(/outline:\s*2px solid var\(--focus\)/);
      }
    }
  });

  it('writes no colour literal in the seed strip canvas code', () => {
    expect(readSource('seeds/StripCanvas.tsx')).not.toMatch(LITERAL);
    expect(readSource('seeds/StripCanvas.tsx')).not.toMatch(NAMED_IN_CODE);
  });

  it('catches a named colour in a stylesheet or in canvas code (the check itself)', () => {
    for (const bad of ['a { color: red; }', 'a { border: 1px solid White; }', 'a { background: var(--x) black; }']) expect(bad).toMatch(NAMED_IN_CSS);
    for (const ok of ['a { color: var(--text); }', 'a { background: transparent; }', 'a { color: currentColor; }', '.green-thing { margin: 0; }']) {
      expect(ok).not.toMatch(NAMED_IN_CSS);
    }
    expect("ctx.fillStyle = 'red';").toMatch(NAMED_IN_CODE);
    expect("const FALLBACK_COLOUR = 'currentColor';").not.toMatch(NAMED_IN_CODE);
  });

  it('gives the scope text beside a badge a left margin, so it does not sit against the badge', () => {
    expect(css).toMatch(/\.badge-scope \{[^}]*margin-left:\s*[\d.]+rem/);
  });

  it('defines the scroll-edge shadow as a token in both themes', () => {
    expect(light['--scroll-shadow']).toBeDefined();
    expect(THEMES.dark['--scroll-shadow']).not.toBe(light['--scroll-shadow']);
  });
});
