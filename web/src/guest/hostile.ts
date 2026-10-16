/**
 * The hostile-string corpus and the markup check shared by every XSS test (RESULTS-EXPLORER-PLAN 7.1). Only tests
 * import this file; it is never part of the page.
 */
import { expect } from 'vitest';
import { CH } from './chars';

const MB = 1024 * 1024;

/** Hostile strings: markup, URLs, template syntax, terminal and bidi tricks, and sizes far past any display limit. */
export const CORPUS: ReadonlyArray<readonly [string, string]> = [
  ['script tag', '<script>alert(1)</script>'],
  ['img onerror', '<img src=x onerror=alert(1)>'],
  ['svg onload', '<svg/onload=alert(1)>'],
  ['attribute break-out', '" onmouseover="alert(1)" x="'],
  ['javascript url', 'javascript:alert(document.cookie)'],
  ['data url', 'data:text/html,<script>alert(1)</script>'],
  ['template syntax', '{{constructor.constructor("alert(1)")()}}'],
  ['bidi override', `safe${CH.RLO}evil.exe`],
  ['bidi isolate', `${CH.LRI}hidden${CH.PDI}`],
  ['line separator', `a${CH.LS}b${CH.PS}c`],
  ['ansi escape', `${CH.ESC}[31mred${CH.ESC}[2J${CH.ESC}]0;title${CH.BEL}`],
  ['nul and del', `a${CH.NUL}b${CH.DEL}c`],
  ['arabic letter mark', `a${CH.ALM}b`],
  ['noncharacter', `a${CH.NONCHAR}b`],
  ['tag characters', `a${CH.TAG_BEGIN}b`],
  ['lone surrogate', `a${CH.LONE_SURROGATE}b`],
  ['hangul fillers', `${CH.HANGUL_CHOSEONG_FILLER}${CH.HANGUL_JUNGSEONG_FILLER}${CH.HANGUL_FILLER}${CH.HALFWIDTH_HANGUL_FILLER}`],
  ['512 bytes', '<b>'.repeat(171)],
  ['long unbroken string', 'W'.repeat(100_000)],
  ['10 MB', '<script>'.repeat((10 * MB) / 8)],
];

/** Control, format and separator characters that must never reach the page raw. */
export const UNSAFE_IN_MARKUP = new RegExp(
  '[\\u0000-\\u0008\\u000b-\\u001f\\u007f\\u0080-\\u009f\\u061c\\u115f\\u1160\\u17b4\\u17b5\\u200b-\\u200f\\u2028-\\u202e\\u2060-\\u206f\\u2800\\u3164\\ufeff\\uffa0\\ufffe\\uffff\\ud800-\\udfff]',
);

/** The only anchors a page may render are the app's own paths, built from ids the server validated. */
const OWN_LINK = /^<a href="\/[A-Za-z0-9_/-]*"( class="[A-Za-z0-9 _-]*")?>$/;

/** Every tag in the output, so guest text can be checked for never having reached markup. */
export const tagsOf = (html: string): string[] => html.match(/<[^>]*>/g) ?? [];

/** Fails if hostile text reached markup: no dangerous element, no payload in any tag, no raw control character. */
export function assertTextOnly(html: string): void {
  const tags = tagsOf(html);
  for (const tag of tags) {
    expect(tag, 'guest text reached markup').not.toMatch(/alert\(|javascript:|onerror|onload|onmouseover|constructor|data:text/i);
    expect(tag).not.toMatch(UNSAFE_IN_MARKUP);
    if (/^<a[\s>]/i.test(tag)) expect(tag, 'a link that is not an app path').toMatch(OWN_LINK);
  }
  expect(html).not.toMatch(UNSAFE_IN_MARKUP);
  const names = new Set(tags.map((t) => /^<\/?([a-zA-Z0-9]+)/.exec(t)?.[1]?.toLowerCase()));
  for (const forbidden of ['script', 'img', 'svg', 'iframe', 'object', 'embed', 'style', 'link', 'form']) {
    expect(names.has(forbidden), `unexpected <${forbidden}>`).toBe(false);
  }
}
