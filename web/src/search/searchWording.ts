import { MAX_SEARCH_CHARS, MIN_SEARCH_CHARS } from './searchText';

/** The words of the search box, in one place. */
export const SEARCH_LABEL = 'Search ids and names';
export const SEARCH_PLACEHOLDER = 'Run, failure, target, seed';
export const RESULTS_LABEL = 'Search results';
export const SEARCHING = 'Searching…';
export const NO_MATCHES = 'No matches.';
export const MORE_NOTE = 'A group marked "more" has further matches than are listed. Type more of the id to narrow it.';

export const TOO_SHORT = `Type at least ${MIN_SEARCH_CHARS} characters.`;
export const BAD_CHARACTERS = 'Use letters, digits and these marks only: _ - . : /';
export const TOO_LONG = `Use at most ${MAX_SEARCH_CHARS} characters.`;

export const matchCount = (n: number): string => (n === 1 ? '1 match.' : `${n} matches.`);

export function tooManySearches(retryAfterMs: number | undefined): string {
  if (retryAfterMs === undefined) return 'Too many searches just now. Try again in a minute.';
  const seconds = Math.max(1, Math.ceil(retryAfterMs / 1000));
  return `Too many searches just now. Try again in ${seconds} ${seconds === 1 ? 'second' : 'seconds'}.`;
}

export const searchFailed = (reason: string): string => `Search failed: ${reason}`;
