import { useCallback } from 'react';
import { FailuresTable } from '../components/FailuresTable';
import { Loading, LoadFailed } from '../components/Notices';
import { UpdatedAt } from '../components/UpdatedAt';
import { FailureFilterBar } from '../lists/FilterBars';
import { failureFiltersFromSearch, failureFiltersToSearch, hasFailureFilters, type FailureFilters } from '../lists/filters';
import { failuresPage } from '../lists/listApi';
import { FAILURES_FILTERED_EMPTY, FAILURES_NONE_READ_YET, loadMoreFailures } from '../lists/listWording';
import { LoadMore } from '../lists/LoadMore';
import { useListPages } from '../lists/useListPages';
import { useAddressFilters } from '../lists/useAddressFilters';
import type { FailureSummary } from '../types';
import { NO_FAILURES_YET } from '../wording';

/** Failures asked for per request; the server allows up to 500. */
const PAGE_SIZE = 50;
const POLL_MS = 5000;

const failureId = (failure: FailureSummary): string => failure.id;

/**
 * The failures the filters let through, newest first. It shows only what `GET /api/failures` sends. The newest page is
 * loaded again every five seconds; older pages are added by asking for them. A change of filters mounts this again.
 */
function FailuresList({ filters }: { filters: FailureFilters }) {
  const loadPage = useCallback(
    (cursor: string | undefined, signal: AbortSignal) => failuresPage({ ...filters, cursor, limit: PAGE_SIZE }, signal),
    [filters],
  );
  const list = useListPages({ loadPage, id: failureId, intervalMs: POLL_MS });
  const rows = list.rows;
  const filtered = hasFailureFilters(filters);
  return (
    <>
      {list.error && <LoadFailed what="failures" message={list.error} />}
      <UpdatedAt at={list.updatedAt} />
      {rows === undefined && list.error === undefined && <Loading what="failures" />}
      {rows?.length === 0 && !list.hasMore && <p className="muted">{filtered ? FAILURES_FILTERED_EMPTY : NO_FAILURES_YET}</p>}
      {rows?.length === 0 && list.hasMore && <p className="muted">{FAILURES_NONE_READ_YET}</p>}
      {rows !== undefined && rows.length > 0 && <FailuresTable failures={rows} />}
      {rows !== undefined && (
        <LoadMore
          label={loadMoreFailures(rows.length)}
          what="failures"
          shown={rows.length}
          hasMore={list.hasMore}
          loading={list.loadingMore}
          onLoadMore={list.loadMore}
          error={list.moreError}
          moreLoads={list.moreLoads}
        />
      )}
    </>
  );
}

export function FailuresPage() {
  // The address mirrors the filters, so a view can be shared or reloaded.
  const [filters, setFilters] = useAddressFilters<FailureFilters>(failureFiltersFromSearch, failureFiltersToSearch);
  const search = failureFiltersToSearch(filters);
  return (
    <>
      <h1>Failures</h1>
      <FailureFilterBar filters={filters} onChange={setFilters} />
      <FailuresList key={search} filters={filters} />
    </>
  );
}
