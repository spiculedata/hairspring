import { useCallback, useEffect, useState } from 'react';
import { api } from '../api';
import { ErrorBoundary } from '../components/ErrorBoundary';
import { filtersFromSearch, filtersToSearch, propertyOf, type SeedFilters } from './filters';
import { replaceAddress } from '../router';
import { usePolling } from '../usePolling';
import { SeedExplorerView, type SeedContext } from './SeedExplorerView';
import { usePropertyLabel, usePropertySearch } from './useProperties';
import { useSeedTrials } from './useSeedTrials';
import { useStrip } from './useStrip';

const TERMINAL = new Set(['passed', 'failed', 'error', 'cancelled']);
const RUN_POLL_MS = 2000;

function useRunHeader(runId: string, live: boolean): SeedContext | undefined {
  const page = usePolling(() => api.properties(runId, { limit: 1 }), RUN_POLL_MS, () => !live);
  const header = page.data?.header;
  return header === undefined
    ? undefined
    : { stoppedAtFirstFailure: header.stopped_at_first_failure, completionLabel: header.completion_label, excluded: header.excluded };
}

function Loaded({ id }: { id: string }) {
  const detail = usePolling(() => api.run(id), RUN_POLL_MS, (d) => TERMINAL.has(d.run.state));
  const run = detail.data?.run;
  const live = run === undefined || !TERMINAL.has(run.state);
  const context = useRunHeader(id, live);
  const [filters, setFilters] = useState<SeedFilters>(() => filtersFromSearch(window.location.search));
  const [fpText, setFpText] = useState(filters.fp ?? '');
  const [searchText, setSearchText] = useState('');
  const strip = useStrip(id, live);
  const trials = useSeedTrials(id, filters, live);
  const search = usePropertySearch(id, searchText);
  const propertyLabel = usePropertyLabel(id, propertyOf(filters)?.idx);

  // The address mirrors the filters, so a view can be shared or reloaded. It is rewritten in place, not pushed.
  useEffect(() => {
    replaceAddress(`${window.location.pathname}${filtersToSearch(filters)}`);
  }, [filters]);

  const onOpen = useCallback((trial: number) => setFilters((f) => ({ ...f, from: trial })), []);

  if (detail.notFound) return <p className="error" role="alert">This run does not exist (it may have been removed).</p>;
  if (detail.error && !run) return <p className="error" role="alert">{detail.error}</p>;
  if (!run) return <p className="muted">Loading…</p>;
  return (
    <SeedExplorerView
      runId={id}
      target={run.target}
      context={context}
      firstFailingSeed={detail.data?.properties?.first_failing_seed}
      live={live}
      strip={strip}
      filters={filters}
      trials={trials}
      filterBar={{ propertyLabel, search, fpText, onSearch: setSearchText, onFpText: setFpText }}
      onFilters={setFilters}
      onOpen={onOpen}
      onLoadMore={trials.loadMore}
    />
  );
}

/** The seed explorer for one run: its strip of judged trials, filters, and a windowed table of trials. */
export function SeedsPage({ id }: { id: string }) {
  return (
    <ErrorBoundary label="Seed explorer">
      <Loaded id={id} />
    </ErrorBoundary>
  );
}
