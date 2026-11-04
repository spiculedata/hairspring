import { describe, expect, it } from 'vitest';
import { DEFAULT_RUN_TAB, RUN_TABS, runTabFromSearch, runTabSearch } from './runTabs';

describe('runTabFromSearch', () => {
  it('reads each tab from ?tab=', () => {
    for (const { id } of RUN_TABS) expect(runTabFromSearch(`?tab=${id}`)).toBe(id);
  });

  it('falls back to the first tab when the parameter is missing, empty or unknown', () => {
    expect(runTabFromSearch('')).toBe(DEFAULT_RUN_TAB);
    expect(runTabFromSearch('?tab=')).toBe(DEFAULT_RUN_TAB);
    expect(runTabFromSearch('?tab=nonsense')).toBe(DEFAULT_RUN_TAB);
    expect(runTabFromSearch('?tab=constructor')).toBe(DEFAULT_RUN_TAB);
    expect(runTabFromSearch('?other=failures')).toBe(DEFAULT_RUN_TAB);
  });

  it('finds the parameter among others', () => {
    expect(runTabFromSearch('?x=1&tab=activity')).toBe('activity');
  });
});

describe('runTabSearch', () => {
  it('leaves the address bare for the first tab and names any other', () => {
    expect(runTabSearch(DEFAULT_RUN_TAB)).toBe('');
    expect(runTabSearch('failures')).toBe('?tab=failures');
  });

  it('round-trips through runTabFromSearch', () => {
    for (const { id } of RUN_TABS) expect(runTabFromSearch(runTabSearch(id))).toBe(id);
  });
});

describe('RUN_TABS', () => {
  it('lists the sections in the order of the design', () => {
    expect(RUN_TABS.map((t) => t.label)).toEqual(['Properties', 'Seeds and trials', 'Failures', 'Activity']);
  });
});
