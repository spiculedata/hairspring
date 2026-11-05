import { useSyncExternalStore } from 'react';
import { api, onTokenChange } from './api';
import type { MeState } from './me';

/**
 * The caller's role, asked of the server once and shared by every component that wants it. The answer is kept across
 * page changes and asked for again only after the token changes. A slow answer for an earlier token is dropped.
 */
const LOADING: MeState = { status: 'loading' };
const UNAVAILABLE: MeState = { status: 'unavailable' };

let state: MeState = LOADING;
let requested = false;
let generation = 0;
const listeners = new Set<() => void>();

function publish(next: MeState): void {
  state = next;
  listeners.forEach((fn) => fn());
}

function load(): void {
  requested = true;
  const mine = ++generation;
  api
    .me()
    .then((me) => {
      if (mine === generation) publish({ status: 'ready', me });
    })
    .catch(() => {
      if (mine === generation) publish(UNAVAILABLE);
    });
}

function forget(): void {
  requested = false;
  generation += 1;
  publish(LOADING);
  if (listeners.size > 0) load();
}

onTokenChange(forget);

function subscribe(fn: () => void): () => void {
  listeners.add(fn);
  if (!requested) load();
  return () => {
    listeners.delete(fn);
  };
}

const snapshot = (): MeState => state;

export function useMe(): MeState {
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}

/** Test helper: forgets the answer and any request still in flight. */
export function resetMeForTests(): void {
  requested = false;
  generation += 1;
  state = LOADING;
}
