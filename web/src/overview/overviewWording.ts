import { formatCount, plural } from '../wording';

/** Every phrase the Overview page generates. Commands are quoted from docs/quickstart.md and the README. */

export const OVERVIEW_HEADING = 'Overview';
export const ATTENTION_HEADING = 'Needs attention';
export const ATTENTION_RUNS_HEADING = 'Failed or errored runs';
export const ATTENTION_FAILURES_HEADING = 'Newest failures';
export const RECENT_RUNS_HEADING = 'Recent runs';
export const WORKERS_HEADING = 'Workers';
export const ALL_RUNS_LINK = 'All runs';
export const ALL_FAILURES_LINK = 'All failures';
export const ALL_WORKERS_LINK = 'Workers page';

/** Said only when both lists loaded and neither has an entry. It states what was looked at, not that anything is fine. */
export const attentionNone = (runsLooked: number): string =>
  `None of the latest ${plural(runsLooked, 'run', 'runs')} failed or errored, and no failures are stored.`;
export const NO_ATTENTION_RUNS = 'No failed or errored run among the latest runs.';
export const NO_ATTENTION_FAILURES = 'No failures stored.';

export const attentionRunsCutOff = (shown: number): string => `Showing ${formatCount(shown)}. The Runs page lists the rest.`;

export const NO_RECENT_RUNS = 'No runs stored.';

export const workersRegistered = (n: number): string => `${plural(n, 'worker', 'workers')} registered`;
export const NO_WORKERS = 'No workers registered.';
export const WORKERS_NOTE = 'Counted from the workers the server lists. This does not say whether a worker is working.';
export const WORKER_LAST_SEEN = 'Most recent contact';

/* First-run guide */

export const GUIDE_HEADING = 'No runs yet';
export const GUIDE_INTRO = 'This server has no stored runs. These are the next steps, as the quick start describes them.';
export const GUIDE_TARGETS_NONE = 'This server lists no targets.';
export const GUIDE_TARGETS_UNKNOWN = 'The targets could not be loaded, so the commands below use a placeholder.';
export const guideTargets = (names: readonly string[], total: number): string => {
  const shown = names.join(', ');
  const more = total > names.length ? ` and ${formatCount(total - names.length)} more` : '';
  return `This server lists ${plural(total, 'target', 'targets')}: ${shown}${more}.`;
};

export const GUIDE_STEP_LIST = 'See which targets the command line knows, including the built-in demos.';
export const GUIDE_STEP_INIT = 'To test your own program, write a starter configuration file, then add a target to it.';
export const GUIDE_STEP_RUN = 'Run a target from hairspring targets with a seed. The server\'s list and the command line\'s may differ. A violation prints the steps and where the bundle was written.';
export const GUIDE_STEP_REPLAY = 'Replay a bundle to run the recorded steps again.';
export const GUIDE_STEP_BROWSER = 'Or start a run from the browser: the Runs page has a form for it.';
export const GUIDE_NOT_A_RESULT = 'Whether a run finds anything depends on the target and the seed. A run that finds nothing says only that those trials did not fail.';
export const GUIDE_PROCESS_NOTE =
  'The process backend, which runs an unmodified program, needs Linux on x86-64 with Hermit installed. docs/quickstart.md in the repository covers it. docs/support-matrix.md says which languages have passed the acceptance test, and on what.';
export const lastGoodLoad = (what: string): string => `Showing the last successful load of ${what}.`;
export const GUIDE_RUNS_LINK = 'Open the Runs page';

export const CMD_TARGETS = 'hairspring targets';
export const CMD_INIT = 'hairspring init';
export const cmdTest = (target: string): string => `hairspring test ${target} --seed 1`;
export const CMD_REPLAY = 'hairspring replay .hairspring/bundles/<bundle>.hairspring';
export const TARGET_PLACEHOLDER = '<target>';
