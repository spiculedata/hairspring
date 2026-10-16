import { useEffect, useState } from 'react';
import { api } from '../api';
import type { PropertyRow } from '../types';
import { seedsApi } from './api';
import type { PropertySearch } from './SeedFilterBar';

/** How long typing pauses before the property list is asked for. */
const SEARCH_DEBOUNCE_MS = 250;
/** Matches shown under the search box. */
const MATCHES_SHOWN = 8;

/**
 * Properties whose id starts with `text`, for choosing one to filter by. Each keystroke aborts the request before it, and
 * a response for older text is dropped. The server matches a prefix of the id and caps its answer.
 */
export function usePropertySearch(runId: string, text: string): PropertySearch {
  const [state, setState] = useState<{ text: string; matches: readonly PropertyRow[]; state: PropertySearch['state'] }>({
    text: '',
    matches: [],
    state: 'idle',
  });
  useEffect(() => {
    if (text === '') {
      setState({ text, matches: [], state: 'idle' });
      return;
    }
    const ctl = new AbortController();
    const timer = setTimeout(() => {
      setState((prev) => ({ ...prev, text, state: 'loading' }));
      api
        .properties(runId, { q: text, sort: 'id', limit: MATCHES_SHOWN }, ctl.signal)
        .then((page) => {
          if (!ctl.signal.aborted) setState({ text, matches: page.properties, state: 'idle' });
        })
        .catch(() => {
          if (!ctl.signal.aborted) setState({ text, matches: [], state: 'error' });
        });
    }, SEARCH_DEBOUNCE_MS);
    return () => {
      clearTimeout(timer);
      ctl.abort();
    };
  }, [runId, text]);
  return { text, matches: state.text === text ? state.matches : [], state: state.text === text ? state.state : 'loading' };
}

/** The id of the property a filter names, read once without scanning for its seeds. `undefined` until it is known. */
export function usePropertyLabel(runId: string, idx: number | undefined): string | undefined {
  const [label, setLabel] = useState<{ idx: number; id: string }>();
  useEffect(() => {
    if (idx === undefined) return;
    const ctl = new AbortController();
    seedsApi
      .property(runId, idx, ctl.signal)
      .then((detail) => {
        if (!ctl.signal.aborted) setLabel({ idx, id: detail.property.guest.id });
      })
      .catch(() => undefined);
    return () => ctl.abort();
  }, [runId, idx]);
  return label !== undefined && label.idx === idx ? label.id : undefined;
}
