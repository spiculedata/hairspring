import { RAW_EVENTS_NOTE } from '../wording';
import { AboutSection } from './AboutSection';
import { rankedCaveats } from './caveats';
import { failingProperties, laneRows } from './model';
import { OutputViewer } from './OutputViewer';
import type { TailState } from './OutputTail';
import { HealthSection, ProcessesSection } from './ProcessesSection';
import { ScheduleSection } from './ScheduleSection';
import { ThreadLanes } from './ThreadLanes';
import { TrialHeader, type RunContext } from './TrialHeader';
import { TrialNav } from './TrialNav';
import { navItems } from './navItems';
import type { StreamName, TrialDetailResponse } from './types';
import { WitnessSection } from './WitnessSection';

export type { RunContext };

export interface TrialViewProps {
  detail: TrialDetailResponse;
  context: RunContext | undefined;
  tails: Partial<Record<StreamName, TailState>>;
  onDownload: (stream: StreamName) => void;
}

const STREAMS: readonly StreamName[] = ['stdout', 'stderr'];

/**
 * The failure page for one trial as a pure function of its data, so it renders the same on the server for tests.
 * Everything from the program under test goes through GuestText in the sections below.
 */
export function TrialView({ detail, context, tails, onDownload }: TrialViewProps) {
  const { trial } = detail;
  const kept = detail.guest.detail;
  const props = kept?.props;
  const failing = props ? failingProperties(props.properties) : [];
  const hasDetail = props !== undefined && kept !== null;
  const [lead, ...rest] = rankedCaveats({
    stoppedAtFirstFailure: context?.stoppedAtFirstFailure === true,
    perturbing: trial.perturbing,
    noDetail: !hasDetail,
    truncated: kept?.truncated === true,
  });
  const streams = STREAMS.filter((s) => detail.output_streams.includes(s) || tails[s] !== undefined);
  return (
    <>
      <TrialHeader runId={detail.run_id} trial={trial} context={context} />
      {lead !== undefined && <p className="note caveat">{lead}</p>}
      <TrialNav items={navItems(hasDetail)} />
      {hasDetail && (
        <>
          <WitnessSection failing={failing} procs={props.procs} />
          <ThreadLanes rows={laneRows(props.procs, failing)} />
          <ProcessesSection procs={props.procs} />
          <HealthSection health={props.health} degraded={trial.degraded} />
        </>
      )}
      <ScheduleSection trial={trial} schedule={detail.guest.schedule} />
      <section aria-labelledby="output-title">
        <h2 id="output-title">Output</h2>
        {streams.length > 0 ? (
          <OutputViewer streams={streams} tails={tails} omittedBytes={detail.output_omitted_bytes} onDownload={onDownload} />
        ) : (
          <p className="muted">No output was kept for this trial.</p>
        )}
        <p className="muted">{RAW_EVENTS_NOTE}</p>
      </section>
      <AboutSection caveats={rest} />
    </>
  );
}
