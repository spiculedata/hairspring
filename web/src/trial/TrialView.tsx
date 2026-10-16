import { DETAIL_TRUNCATED_NOTE, NO_DETAIL_NOTE, RAW_EVENTS_NOTE } from '../wording';
import { failingProperties, laneRows } from './model';
import { OutputTail, type TailState } from './OutputTail';
import { HealthSection, ProcessesSection } from './ProcessesSection';
import { ScheduleSection } from './ScheduleSection';
import { ThreadLanes } from './ThreadLanes';
import { TrialHeader, type RunContext } from './TrialHeader';
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
  return (
    <>
      <TrialHeader runId={detail.run_id} trial={trial} context={context} />
      {props === undefined || kept === null ? (
        <p className="note">{NO_DETAIL_NOTE}</p>
      ) : (
        <>
          {kept.truncated && <p className="note">{DETAIL_TRUNCATED_NOTE}</p>}
          <WitnessSection failing={failing} procs={props.procs} />
          <ThreadLanes rows={laneRows(props.procs, failing)} />
          <ProcessesSection procs={props.procs} />
          <HealthSection health={props.health} degraded={trial.degraded} />
        </>
      )}
      <ScheduleSection trial={trial} schedule={detail.guest.schedule} />
      <section aria-labelledby="output-title">
        <h2 id="output-title">Output</h2>
        {STREAMS.filter((s) => detail.output_streams.includes(s) || tails[s] !== undefined).map((s) => (
          <OutputTail
            key={s}
            stream={s}
            state={tails[s] ?? { status: 'absent' }}
            omittedBytes={detail.output_omitted_bytes[s]}
            onDownload={onDownload}
          />
        ))}
        {detail.output_streams.length === 0 && Object.keys(tails).length === 0 && <p className="muted">No output was kept for this trial.</p>}
        <p className="muted">{RAW_EVENTS_NOTE}</p>
      </section>
    </>
  );
}
