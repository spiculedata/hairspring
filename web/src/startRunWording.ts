/**
 * What the start-run form says. The sentences about search and sweep are the repository's own (docs/architecture.md,
 * "Sweep mode", and the quickstart): a search ends at its first confirmed failure and has no rate; a sweep runs every
 * trial of its budget and a rate can be stated for the trials it ran. Nothing here says a sweep shows that a failure
 * cannot happen.
 */
export const RUN_TYPE_LEGEND = 'Run type';
export const SEARCH_LABEL = 'Find a failure (search)';
export const SWEEP_LABEL = 'Measure how often (sweep)';

export const SEARCH_EXPLAINED =
  'A search ends at its first confirmed failure, so its trial count was decided by its outcome and it has no rate.';

export const SWEEP_EXPLAINED = [
  'A sweep runs the same target many times and can take a long time. It ends at its trial, step or wall-clock budget, whichever comes first.',
  'It runs every trial of its budget, continues after a failure and stores every trial, so a rate can be stated for the trials it ran.',
  'A sweep with no failure says only that those trials did not fail.',
] as const;

export const BUDGET_HEADING = 'Budget the server will enforce';
export const TRIALS_LABEL = 'Trials (1 to 10,000)';
export const TRIALS_PLACEHOLDER = "the target's own";
export const START_RUN = 'Start run';
export const START_SWEEP = 'Start sweep';
export const STARTING = 'Starting…';
