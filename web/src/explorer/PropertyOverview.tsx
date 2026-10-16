import type { PropertyPage, PropertyQuery, PropertyRow } from '../types';
import { loadMore, propertiesTitle } from '../wording';
import { PropertyFilters } from './PropertyFilters';
import { PropertyTable } from './PropertyTable';
import { RunSummary } from './RunSummary';
import { UnmetSection } from './UnmetSection';

interface ViewProps {
  /** The first page, which carries the header, claim and counts. */
  page: PropertyPage;
  /** Every row loaded so far, across pages. */
  rows: readonly PropertyRow[];
  query: PropertyQuery;
  onQuery: (next: PropertyQuery) => void;
  /** Absent when there is no further page. */
  onLoadMore: (() => void) | undefined;
  loadingMore: boolean;
}

/** The run overview as a pure function of its data, so it renders the same on the server for tests. */
export function PropertyOverviewView({ page, rows, query, onQuery, onLoadMore, loadingMore }: ViewProps) {
  const allProperties = Object.values(page.counts_by_status).reduce<number>((sum, n) => sum + (n ?? 0), 0);
  return (
    <section aria-labelledby="properties-title">
      <RunSummary page={page} />
      <h2 id="properties-title">{propertiesTitle(allProperties)}</h2>
      <PropertyFilters query={query} onQuery={onQuery} />
      <PropertyTable properties={rows} />
      {onLoadMore && (
        <p>
          <button type="button" onClick={onLoadMore} disabled={loadingMore}>
            {loadMore(rows.length, page.total)}
          </button>
        </p>
      )}
      <UnmetSection rows={rows} unmetTotal={page.counts_by_status.unmet ?? 0} header={page.header} />
    </section>
  );
}
