import { useCallback } from 'react';
import { api } from '../api';
import { LoadFailed, Loading } from '../components/Notices';
import { StartRunForm } from '../components/StartRunForm';
import { RunsTable } from '../components/RunsTable';
import { UpdatedAt } from '../components/UpdatedAt';
import { startAccess } from '../me';
import { useMe } from '../meStore';
import { RunFilterBar } from '../lists/FilterBars';
import { hasRunFilters, runFiltersFromSearch, runFiltersToSearch, type RunFilters } from '../lists/filters';
import { runsPage } from '../lists/listApi';
import { loadMoreRuns } from '../lists/listWording';
import { LoadMore } from '../lists/LoadMore';
import { useListPages } from '../lists/useListPages';
import { useAddressFilters } from '../lists/useAddressFilters';
import { navigate } from '../router';
import type { Run } from '../types';
import { usePolling } from '../usePolling';

/** Runs asked for per request; the server allows up to 500. */
const PAGE_SIZE = 50;
const POLL_MS = 2000;

const runId = (run: Run): string => run.id;

function NewRun() {
  const targets = usePolling(api.targets, 60_000);
  const access = startAccess(useMe());
  return <StartRunForm targets={targets.data ?? []} start={api.startRun} onStarted={navigate} access={access} />;
}

/**
 * The runs the filters let through, newest first. The newest page is loaded again every two seconds so a run's state
 * stays current; older pages are added by asking for them. A change of filters mounts this again (it is given the
 * filters as its key), which starts a new list.
 */
function RunsList({ filters }: { filters: RunFilters }) {
  const loadPage = useCallback(
    (cursor: string | undefined, signal: AbortSignal) => runsPage({ ...filters, cursor, limit: PAGE_SIZE }, signal),
    [filters],
  );
  const list = useListPages({ loadPage, id: runId, intervalMs: POLL_MS, rowsLeave: filters.state !== undefined });
  const rows = list.rows;
  return (
    <>
      {list.error && <LoadFailed what="runs" message={list.error} />}
      <UpdatedAt at={list.updatedAt} />
      {rows === undefined && list.error === undefined && <Loading what="runs" />}
      {rows !== undefined && <RunsTable runs={rows} filtered={hasRunFilters(filters)} />}
      {rows !== undefined && (
        <LoadMore
          label={loadMoreRuns(rows.length)}
          what="runs"
          shown={rows.length}
          hasMore={list.hasMore}
          loading={list.loadingMore}
          endReached={list.endReached}
          onLoadMore={list.loadMore}
          error={list.moreError}
          moreLoads={list.moreLoads}
        />
      )}
    </>
  );
}

export function Runs() {
  // The address mirrors the filters, so a view can be shared or reloaded.
  const [filters, setFilters] = useAddressFilters<RunFilters>(runFiltersFromSearch, runFiltersToSearch);
  const search = runFiltersToSearch(filters);
  return (
    <>
      <h1>Runs</h1>
      <NewRun />
      <RunFilterBar filters={filters} onChange={setFilters} />
      <RunsList key={search} filters={filters} />
    </>
  );
}
