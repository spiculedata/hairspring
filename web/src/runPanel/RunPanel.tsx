import { StatusBadge } from '../components/StatusBadge';
import type { PropertyPage, RateCellData, RunDetail } from '../types';
import './runPanel.css';
import { runPanelModel, type RunPanelInput } from './runPanelModel';
import { DOES_NOT_SHOW_HEADING, OUTCOME_LABEL, PANEL_LABEL, SHOWS_HEADING } from './runPanelWording';

/** The rate cells of the failed properties on the loaded page. Property ids and messages are not read: they are guest text. */
function failedRates(page: PropertyPage | undefined): RateCellData[] {
  // The loaded rows are the first page under the reader's filters, so a rate is named only when the server counts one
  // failed property in all and the page holds it in full.
  if (!page || page.counts_by_status.violation !== 1 || page.next_cursor !== null) return [];
  return page.properties.filter((p) => p.status === 'violation' && p.rate.kind === 'rate' && p.rate.n > 0).map((p) => p.rate);
}

export function panelInput(detail: RunDetail, page: PropertyPage | undefined): RunPanelInput {
  const { run } = detail;
  return {
    state: run.state,
    trials: run.trials,
    completion: page?.header.completion,
    completionLabel: page?.header.completion_label,
    sweep: detail.sweep,
    sweepPartial: detail.sweep_partial === true,
    failureCount: detail.failures.length,
    firstFailingSeed: detail.properties?.first_failing_seed,
    failedRates: failedRates(page),
    failedPropertyCount: page?.counts_by_status.violation,
    findings: detail.failures.map((f) => f.finding?.status),
    excluded: page?.header.excluded,
    ingestIncomplete: page?.header.ingest_incomplete,
    unverifiedWorkerData: page?.header.unverified_worker_data,
  };
}

function Sentences({ heading, lines }: { heading: string; lines: readonly string[] }) {
  return (
    <div>
      <h3>{heading}</h3>
      <ul>
        {lines.map((line) => (
          <li key={line}>{line}</li>
        ))}
      </ul>
    </div>
  );
}

/**
 * What this run shows and what it does not, in sentences built only from the server's run detail and property page.
 * The sentences are in runPanelModel.ts; the words are in runPanelWording.ts.
 */
export function RunPanel({ detail, page }: { detail: RunDetail; page: PropertyPage | undefined }) {
  const { shows, doesNotShow } = runPanelModel(panelInput(detail, page));
  return (
    <section className="card run-panel" aria-label={PANEL_LABEL}>
      <p className="run-panel-outcome">
        <span className="muted">{OUTCOME_LABEL}: </span>
        <StatusBadge kind="run" state={detail.run.state} trials={detail.run.trials} completion={page?.header.completion} />
      </p>
      <div className="run-panel-columns">
        <Sentences heading={SHOWS_HEADING} lines={shows} />
        <Sentences heading={DOES_NOT_SHOW_HEADING} lines={doesNotShow} />
      </div>
    </section>
  );
}
