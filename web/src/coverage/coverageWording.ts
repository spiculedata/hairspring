import type { PanelKey } from './coverageModel';

const COUNT = new Intl.NumberFormat('en-GB');

export const COVERAGE_TITLE = 'Coverage over trials';

export const COVERAGE_NOTE =
  'Cumulative counts of distinct things seen so far, by trial number. A count never falls. A flat line means nothing new was seen in that stretch; it does not mean nothing is left to find.';

export const COVERAGE_EMPTY = 'No trial has been judged yet, so there is no coverage to draw.';

/** Said instead of the charts when every count stayed at zero: three flat lines say nothing a sentence does not. */
export const coverageNothingSeen = (trials: number): string =>
  `Nothing was seen in any of the ${COUNT.format(trials)} ${trials === 1 ? 'trial' : 'trials'}: no property ids were reached or seen true, and no distinct schedules were counted.`;

export const AXIS_LABEL = 'Trial number';
export const SHOW_TABLE = 'Show as a table';

export const PANEL_LABELS: Readonly<Record<PanelKey, string>> = {
  ids_hit: 'Property ids reached',
  ids_true: 'Property ids seen true',
  schedules: 'Distinct schedules',
};


/** Said when points were left out. The server thins a very long series; the page thins again to stay readable. */
export function downsampledNote(shown: number, total: number, serverThinned: boolean): string {
  if (shown < total) {
    const earlier = serverThinned ? ' The server had already thinned the series.' : '';
    return `Downsampled: showing ${COUNT.format(shown)} of ${COUNT.format(total)} points. The table lists the same points as the chart. The figures under each heading come from every point, so the trial named there may not be one of those drawn.${earlier}`;
  }
  if (serverThinned) return 'Downsampled by the server: the series was thinned before it was sent, so the line has less detail than the run.';
  return '';
}
