/**
 * What the filter bars and the "Load more" controls of the Runs and Failures lists say. A run the server calls `passed`
 * is labelled "Finished", as the list's own rows label it, because the list holds no completion to say more.
 */
import { FINDING_LABELS } from '../confirmationWording';
import { FINISHED } from '../runOutcome';
import { runStateEntry } from '../statusMap';
import { SEARCH_LABEL, SWEEP_LABEL } from '../startRunWording';
import { formatCount } from '../wording';
import { MAX_PROPERTY_PREFIX_BYTES, MAX_TARGET_BYTES, type FilterProblem, type RunModeFilter } from './filters';
import type { FindingStatus, RunState } from '../types';

export const FILTER_ALL = 'All';
export const FILTERS_CLEAR = 'Clear filters';

export const RUN_FILTERS_LABEL = 'Filter runs';
export const RUN_STATE_LABEL = 'State';
export const RUN_MODE_LABEL = 'Run type';
export const RUN_TARGET_LABEL = 'Target';

export const FAILURE_FILTERS_LABEL = 'Filter failures';
export const FAILURE_RUN_LABEL = 'Run id';
export const FAILURE_PROPERTY_LABEL = 'Property id prefix';
export const FAILURE_STATUS_LABEL = 'State';

export const RUNS_FILTERED_EMPTY = 'No runs match these filters.';
export const FAILURES_FILTERED_EMPTY = 'No failures match these filters.';
/** A state filter reads a bounded number of failures per request, so a page can be empty while older failures remain to be read. */
export const FAILURES_NONE_READ_YET = 'None of the failures read so far match. Load more to read further back.';

export const runStateOption = (state: RunState): string => (state === 'passed' ? FINISHED : runStateEntry(state).label);
export const runModeOption = (mode: RunModeFilter): string => (mode === 'search' ? SEARCH_LABEL : SWEEP_LABEL);
export const findingOption = (status: FindingStatus): string => FINDING_LABELS[status];

export const PROPERTY_FILTER_HINT = 'Failures of runs in which a property whose id starts with this text is stored as failed.';
export const STATUS_FILTER_NOTE =
  'A state is worked out from each failure’s stored checks, so filtering by it can take longer on a long list. A page can hold fewer failures than usual: load more to carry on.';

export function problemText(problem: FilterProblem): string {
  switch (problem) {
    case 'target':
      return `A target name has 1 to ${MAX_TARGET_BYTES} bytes (a character outside ASCII takes more than one) and no control characters.`;
    case 'property':
      return `A property id prefix has at most ${MAX_PROPERTY_PREFIX_BYTES} bytes and no control characters.`;
    case 'run':
      return 'A run id is run_ followed by 16 hexadecimal digits.';
  }
}

export const loadMoreRuns = (shown: number): string => `Load more runs (${formatCount(shown)} shown)`;
export const loadMoreFailures = (shown: number): string => `Load more failures (${formatCount(shown)} shown)`;
export const allShown = (shown: number, what: 'runs' | 'failures'): string => `All ${formatCount(shown)} ${what} are shown.`;
/** The server reads a list down to a fixed depth; a filter reaches what lies beyond it. */
export const LIST_DEPTH_REACHED = 'That is as far back as this list can be read. Narrow it with a filter to see older ones.';
export const LOADING_MORE = 'Loading more…';
export const shownAnnouncement = (shown: number, what: 'runs' | 'failures'): string => `${formatCount(shown)} ${what} shown.`;
export const loadMoreFailed = (what: 'runs' | 'failures', message: string): string => `Could not load more ${what}: ${message}`;
