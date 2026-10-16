import { ErrorBoundary } from '../components/ErrorBoundary';
import { ErrorLine } from '../components/ErrorLine';
import { GuestText } from '../components/GuestText';
import { Link } from '../components/Layout';
import { FirstFailingSeed } from '../explorer/FirstFailingSeed';
import { runPath } from '../router';
import type { Excluded } from '../types';
import {
  continueReading,
  excludedText,
  LOADING_TRIALS,
  loadMoreTrials,
  NO_TRIALS,
  NO_TRIALS_MATCH,
  runningNote,
  scannedNoMatch,
  SEEDS_TITLE,
  stoppedRunNote,
} from '../wording';
import { withProperty, type SeedFilters } from './filters';
import { SeedFilterBar, type PropertySearch } from './SeedFilterBar';
import { StripSection } from './StripSection';
import type { StripModel } from './useStrip';
import type { SeedTrialsModel } from './useSeedTrials';
import { TrialTable } from './TrialTable';

const TARGET_CHARS = 80;

/** What the run says about how it ended and what it left out, from the first row of its property page. */
export interface SeedContext {
  readonly stoppedAtFirstFailure: boolean;
  readonly completionLabel: string;
  readonly excluded: Excluded;
}

export interface SeedExplorerViewProps {
  runId: string;
  target: string;
  context: SeedContext | undefined;
  /** The lowest failing seed of the run, as a decimal string. */
  firstFailingSeed: string | null | undefined;
  live: boolean;
  strip: StripModel;
  filters: SeedFilters;
  trials: Pick<SeedTrialsModel, 'rows' | 'cursor' | 'loaded' | 'loadingMore' | 'error'>;
  filterBar: {
    propertyLabel: string | undefined;
    search: PropertySearch;
    fpText: string;
    onSearch: (text: string) => void;
    onFpText: (text: string) => void;
  };
  onFilters: (next: SeedFilters) => void;
  /** Open the table at a trial. */
  onOpen: (trial: number) => void;
  onLoadMore: () => void;
}

/** How many rows the list has, when that is known: an unfiltered list of a run whose strip has loaded. */
export function knownTotal(filters: SeedFilters, judged: number, stripLoading: boolean): number | undefined {
  const { from, ...narrowing } = filters;
  if (stripLoading || Object.values(narrowing).some((v) => v !== undefined)) return undefined;
  return Math.max(0, judged - (from ?? 0));
}

function TrialsBody(props: SeedExplorerViewProps) {
  const { runId, trials, filters, strip, onFilters, onLoadMore } = props;
  const judged = strip.strip.codes.length;
  const total = knownTotal(filters, judged, strip.loading);
  const onFilter = (p: Partial<SeedFilters>) =>
    onFilters(p.fails !== undefined ? { ...withProperty(filters, 'fails', p.fails), from: undefined } : { ...filters, ...p, from: undefined });
  const more = trials.cursor;
  if (trials.error !== undefined && trials.rows.length === 0) return <ErrorLine message={trials.error} />;
  if (!trials.loaded) return <p className="muted">{LOADING_TRIALS}</p>;
  if (trials.rows.length === 0 && more === null) {
    return <p className="muted">{judged === 0 && Object.keys(filters).length === 0 ? NO_TRIALS : NO_TRIALS_MATCH}</p>;
  }
  return (
    <>
      {trials.error !== undefined && <ErrorLine message={trials.error} />}
      {trials.rows.length === 0 && more !== null && <p className="muted">{scannedNoMatch(Number(more))}</p>}
      <TrialTable
        key={JSON.stringify(filters)}
        runId={runId}
        rows={trials.rows}
        total={total}
        highlightSeq={filters.from}
        onFilter={onFilter}
        onNearEnd={() => {
          if (more !== null && !trials.loadingMore) onLoadMore();
        }}
      />
      {more !== null && (
        <p>
          <button type="button" onClick={onLoadMore} disabled={trials.loadingMore}>
            {trials.rows.length === 0 ? continueReading(Number(more)) : loadMoreTrials(trials.rows.length, total)}
          </button>
        </p>
      )}
    </>
  );
}

/** The seed explorer as a pure function of its data, so it renders the same on the server for tests. */
export function SeedExplorerView(props: SeedExplorerViewProps) {
  const { runId, target, context, firstFailingSeed, live, strip, filters, filterBar, onFilters, onOpen } = props;
  const judged = strip.strip.codes.length;
  const lastTrial = strip.strip.nextSeq > 0 ? strip.strip.nextSeq - 1 : undefined;
  const excluded = context === undefined ? undefined : excludedText(context.excluded);
  return (
    <>
      <p>
        <Link to={runPath(runId)}>← Run</Link>
      </p>
      <h1>
        {SEEDS_TITLE}: <GuestText text={target} max={TARGET_CHARS} expandable={false} />
      </h1>
      {context !== undefined && <p className="muted">{context.completionLabel}</p>}
      {context?.stoppedAtFirstFailure === true && <p className="note">{stoppedRunNote(lastTrial)}</p>}
      {live && <p className="muted">{runningNote(judged)}</p>}
      {excluded !== undefined && <p className="excluded">{excluded}</p>}
      <FirstFailingSeed runId={runId} seed={firstFailingSeed} />
      <ErrorBoundary label="Strip">
        <StripSection strip={strip.strip} loading={strip.loading} error={strip.error} onOpen={onOpen} />
      </ErrorBoundary>
      <section aria-labelledby="trials-title">
        <h2 id="trials-title">Trials</h2>
        <SeedFilterBar filters={filters} onChange={onFilters} {...filterBar} />
        <ErrorBoundary label="Trial table">
          <TrialsBody {...props} />
        </ErrorBoundary>
      </section>
    </>
  );
}
