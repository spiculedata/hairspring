import { ApiError } from './api';
import { formatSeedInput } from './format';
import type { Budget, Target } from './types';

/** The limits the server clamps a run's budget to (`Budget::clamp` in hairspring-engine). The form refuses past them. */
export const MAX_TRIALS = 10_000;
export const MAX_STEPS = 50_000_000;
export const MAX_WALL_SECS = 600;

export type RunMode = 'search' | 'sweep';

/** The body of `POST /api/runs`. `budget`, when sent, replaces the target's budget whole. */
export interface StartRunBody {
  readonly target: string;
  readonly seed?: number;
  readonly mode: RunMode;
  readonly budget?: Budget;
}

export interface StartFormInput {
  readonly target: Target;
  readonly mode: RunMode;
  readonly seedText: string;
  /** Only read for a sweep. */
  readonly trialsText: string;
}

export interface StartFormErrors {
  readonly seed?: string;
  readonly trials?: string;
}

export type StartFormResult = { readonly ok: true; readonly body: StartRunBody } | { readonly ok: false; readonly errors: StartFormErrors };

export const TRIALS_NOT_WHOLE = 'Trials must be a whole number';
export const TRIALS_OUT_OF_RANGE = 'Trials must be between 1 and 10,000';

type TrialsParse = { readonly ok: true; readonly value: number | undefined } | { readonly ok: false; readonly message: string };

function parseTrials(text: string): TrialsParse {
  const t = text.trim();
  if (t === '') return { ok: true, value: undefined };
  if (!/^\d+$/.test(t)) return { ok: false, message: TRIALS_NOT_WHOLE };
  const n = Number(t);
  return n >= 1 && n <= MAX_TRIALS ? { ok: true, value: n } : { ok: false, message: TRIALS_OUT_OF_RANGE };
}

/** The budget the server will hold a run to: the target's, with the chosen trials for a sweep, within the server's caps. */
export function enforcedBudget(budget: Budget, mode: RunMode, trials?: number): Budget {
  return {
    max_steps: Math.min(budget.max_steps, MAX_STEPS),
    max_trials: Math.min(mode === 'sweep' && trials !== undefined ? trials : budget.max_trials, MAX_TRIALS),
    wall_secs: Math.min(budget.wall_secs, MAX_WALL_SECS),
  };
}

/**
 * The request for the form as filled in, or the error of each field that is wrong. A budget is sent only for a sweep
 * with trials given, and then as a whole: the server replaces the target's budget with what it is sent, filling any
 * missing field with its default, so sending trials alone would change the steps and seconds too.
 */
export function buildStartRun({ target, mode, seedText, trialsText }: StartFormInput): StartFormResult {
  const seed = formatSeedInput(seedText);
  const trials = mode === 'sweep' ? parseTrials(trialsText) : ({ ok: true, value: undefined } as const);
  if (!seed.ok || !trials.ok) {
    return { ok: false, errors: { ...(seed.ok ? {} : { seed: seed.message }), ...(trials.ok ? {} : { trials: trials.message }) } };
  }
  return {
    ok: true,
    body: {
      target: target.name,
      ...(seed.value === undefined ? {} : { seed: seed.value }),
      mode,
      ...(trials.value === undefined ? {} : { budget: { ...target.budget, max_trials: trials.value } }),
    },
  };
}

/** Why a start failed, by cause, in words that say what the reader can do. The server's own reason is quoted, never replaced. */
export function startErrorText(e: unknown): string {
  if (!(e instanceof ApiError)) return `Could not reach the server: ${e instanceof Error ? e.message : String(e)}.`;
  if (e.status === 401) return 'The server did not accept the token, so this browser cannot start runs. Enter a token that can.';
  if (e.status === 403) return `This token cannot start runs. The server says: ${e.message}.`;
  if (e.status === 429) return `The server is limiting requests: ${e.message}.`;
  if (e.status >= 500) return `The server failed to start the run: ${e.message}.`;
  return `The server did not accept this run: ${e.message}.`;
}
