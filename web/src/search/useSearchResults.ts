import { useEffect, useRef, useState } from 'react';
import { ApiError } from '../api';
import { isNetworkFailure } from '../pollPolicy';
import { searchRequest, type SearchResults } from './searchApi';
import { checkSearchText } from './searchText';
import { SEARCH_OFFLINE, searchFailed, tooManySearches } from './searchWording';

/** How long typing must pause before a search is sent. */
export const SEARCH_DEBOUNCE_MS = 250;

export type SearchView =
  /** Nothing typed. */
  | { readonly status: 'idle' }
  /** The text cannot be searched yet or at all; nothing is sent. */
  | { readonly status: 'hint'; readonly problem: 'short' | 'characters' | 'long' }
  /** A search is waiting or in flight. `previous` is the last answer, still on screen until the new one arrives. */
  | { readonly status: 'busy'; readonly previous?: SearchResults }
  | { readonly status: 'done'; readonly results: SearchResults }
  | { readonly status: 'error'; readonly message: string };

type Outcome =
  | { readonly text: string; readonly results: SearchResults }
  | { readonly text: string; readonly message: string };

export interface SearchModel {
  readonly view: SearchView;
}

const messageOf = (e: unknown): string => (e instanceof Error ? e.message : String(e));

/**
 * Searches for `raw` after typing pauses, one request at a time: a newer text cancels the older request and an answer
 * for text that is no longer typed is dropped. Text the server would refuse is not sent. After a 429 nothing is sent
 * until the time the server gave has passed, so typing on cannot spend the limit further. `onAbsent` is called when the
 * server has no search endpoint (404).
 */
export function useSearchResults(raw: string, onAbsent: () => void, now: () => number = Date.now): SearchModel {
  const checked = checkSearchText(raw);
  const text = checked.kind === 'ok' ? checked.text : undefined;
  const [outcome, setOutcome] = useState<Outcome>();
  const last = useRef<SearchResults | undefined>(undefined);
  const [retry, setRetry] = useState(0);
  const blockedUntil = useRef(0);
  const unblock = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const absent = useRef(onAbsent);
  absent.current = onAbsent;

  useEffect(() => {
    if (text === undefined) return;
    const controller = new AbortController();
    const wait = Math.max(SEARCH_DEBOUNCE_MS, blockedUntil.current - now());
    const timer = setTimeout(() => {
      searchRequest(text, controller.signal).then(
        (results) => {
          if (controller.signal.aborted) return;
          last.current = results;
          setOutcome({ text, results });
        },
        (e: unknown) => {
          if (controller.signal.aborted) return;
          if (e instanceof ApiError && e.status === 404) return absent.current();
          if (e instanceof ApiError && e.status === 429) {
            const pause = e.retryAfterMs ?? 60_000;
            blockedUntil.current = now() + pause;
            unblock.current = setTimeout(() => {
              setOutcome(undefined);
              setRetry((n) => n + 1);
            }, pause);
            return setOutcome({ text, message: tooManySearches(e.retryAfterMs) });
          }
          setOutcome({ text, message: isNetworkFailure(e) ? SEARCH_OFFLINE : searchFailed(messageOf(e)) });
        },
      );
    }, wait);
    return () => {
      clearTimeout(timer);
      clearTimeout(unblock.current);
      controller.abort();
    };
  }, [text, now, retry]);

  if (checked.kind === 'empty') return { view: { status: 'idle' } };
  if (checked.kind !== 'ok') return { view: { status: 'hint', problem: checked.kind === 'short' ? 'short' : checked.problem } };
  if (outcome?.text === checked.text) {
    return { view: 'results' in outcome ? { status: 'done', results: outcome.results } : { status: 'error', message: outcome.message } };
  }
  const waiting = blockedUntil.current - now();
  if (waiting > 0) return { view: { status: 'error', message: tooManySearches(waiting) } };
  return { view: { status: 'busy', previous: last.current } };
}
