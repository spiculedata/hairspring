import { PropertyOverviewView } from './PropertyOverview';
import type { PropertyOverviewModel } from './usePropertyOverview';

/** The overview wired to its loaded data: loading, error and ready states. */
export function PropertyOverviewPanel({ model }: { model: PropertyOverviewModel }) {
  const { page, rows, query, setQuery, loadMore, loadingMore, error } = model;
  if (!page) {
    return error ? (
      <p className="error" role="alert">
        Property table unavailable: {error}
      </p>
    ) : (
      <p className="muted">Loading properties…</p>
    );
  }
  return (
    <>
      {error && (
        <p className="error" role="alert">
          Property table could not be refreshed: {error}
        </p>
      )}
      <PropertyOverviewView page={page} rows={rows} query={query} onQuery={setQuery} onLoadMore={loadMore} loadingMore={loadingMore} />
    </>
  );
}
