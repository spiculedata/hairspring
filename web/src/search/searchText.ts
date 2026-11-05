/** Fewest characters searched: one character matches almost every id, so the box waits for a second. */
export const MIN_SEARCH_CHARS = 2;
/** Most characters the server accepts in `q`. */
export const MAX_SEARCH_CHARS = 64;

const ALLOWED = /^[A-Za-z0-9_.:/-]+$/;

export type SearchText =
  | { readonly kind: 'empty' }
  | { readonly kind: 'short' }
  | { readonly kind: 'invalid'; readonly problem: 'characters' | 'long' }
  | { readonly kind: 'ok'; readonly text: string };

/**
 * What the typed text asks for. It mirrors the server's rule (ASCII letters, digits and `_ - . : /`, 1 to 64
 * characters) so that text the server would refuse is not sent. The spaces around a pasted id are dropped.
 */
export function checkSearchText(raw: string): SearchText {
  const text = raw.trim();
  if (text === '') return { kind: 'empty' };
  if (!ALLOWED.test(text)) return { kind: 'invalid', problem: 'characters' };
  if (text.length > MAX_SEARCH_CHARS) return { kind: 'invalid', problem: 'long' };
  if (text.length < MIN_SEARCH_CHARS) return { kind: 'short' };
  return { kind: 'ok', text };
}
