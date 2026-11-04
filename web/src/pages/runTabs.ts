export type RunTabId = 'properties' | 'seeds' | 'failures' | 'activity';

export const RUN_TABS: readonly { id: RunTabId; label: string }[] = [
  { id: 'properties', label: 'Properties' },
  { id: 'seeds', label: 'Seeds and trials' },
  { id: 'failures', label: 'Failures' },
  { id: 'activity', label: 'Activity' },
];

export const DEFAULT_RUN_TAB: RunTabId = 'properties';

/** The tab a run page's query string names (`?tab=failures`); anything else shows the first tab. */
export function runTabFromSearch(search: string): RunTabId {
  const named = new URLSearchParams(search).get('tab');
  return RUN_TABS.find((t) => t.id === named)?.id ?? DEFAULT_RUN_TAB;
}

/** The query string that selects `tab`: empty for the first tab, so the plain run address keeps working. */
export function runTabSearch(tab: RunTabId): string {
  return tab === DEFAULT_RUN_TAB ? '' : `?tab=${tab}`;
}
