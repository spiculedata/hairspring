/// <reference types="node" />
import { readFileSync } from 'node:fs';

/**
 * The text of a file under `src/`, for tests that check a number in code against the stylesheet it must agree with.
 * (Vitest does not give stylesheets to `?raw` imports, so the file is read from disk.) Used by tests only.
 */
export function readSource(relativeToSrc: string): string {
  return readFileSync(new URL(`./${relativeToSrc}`, import.meta.url), 'utf8');
}
