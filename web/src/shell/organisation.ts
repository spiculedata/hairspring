/** Longest organisation name the server sends, in characters. */
export const MAX_ORGANISATION_CHARS = 80;

/** Control characters, and characters that reorder or hide text (bidirectional controls, zero-width and byte-order marks)
 * and the line and paragraph separators U+2028 and U+2029. Keep it in step with `is_unsafe_display_char` in the server. */
const UNSAFE_CHARACTER = /[\p{Cc}؜​-‏\u2028\u2029‪-‮⁠-⁩﻿]/u;

/**
 * The organisation name in a `GET /api/health` answer, or `undefined` when there is none to show. The server checks the
 * name before it serves it; this checks again, because the page prints whatever arrives: only text of 1 to 80
 * characters with nothing that could hide or reorder it is used. The name is never treated as markup.
 */
export function organisationName(body: unknown): string | undefined {
  if (typeof body !== 'object' || body === null || Array.isArray(body)) return undefined;
  const raw = (body as Record<string, unknown>).organisation;
  if (typeof raw !== 'string') return undefined;
  const name = raw.trim();
  if (name === '' || [...name].length > MAX_ORGANISATION_CHARS || UNSAFE_CHARACTER.test(name)) return undefined;
  return name;
}
