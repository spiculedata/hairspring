import type { ReactNode } from 'react';
import { GuestText } from '../components/GuestText';
import { formatCount, NO_SCHEDULE_NOTE } from '../wording';
import type { ScheduleData, TrialRow } from './types';

const FIELD_CHARS = 400;
const SLICES_CHARS = 8 * 1024;

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <>
      <dt>{label}</dt>
      <dd>{children}</dd>
    </>
  );
}

/**
 * The scheduler summary's deterministic fields (turns, syscalls, virtual time, timeslices), the schedule fingerprint and
 * the row's own figures. The fields are as Hermit wrote them; none is wall-clock time.
 */
export function ScheduleSection({ trial, schedule }: { trial: TrialRow; schedule: ScheduleData | null }) {
  return (
    <section aria-labelledby="schedule-title">
      <h2 id="schedule-title">Schedule</h2>
      {schedule === null && <p className="muted">{NO_SCHEDULE_NOTE}</p>}
      <dl className="facts">
        {schedule !== null && <Field label="Scheduler turns">{formatCount(schedule.sched_turns)}</Field>}
        <Field label="Syscalls">{formatCount(schedule?.syscalls ?? trial.syscalls)}</Field>
        <Field label="Fingerprint">{trial.schedule_fp === null ? 'not recorded' : <code>{trial.schedule_fp}</code>}</Field>
        {schedule?.virttime_elapsed != null && (
          <Field label="Virtual time elapsed">
            <GuestText text={schedule.virttime_elapsed} max={FIELD_CHARS} expandable={false} />
          </Field>
        )}
        {schedule?.timeslice_stats != null && (
          <Field label="Timeslice statistics">
            <GuestText text={schedule.timeslice_stats} max={FIELD_CHARS} expandable={false} />
          </Field>
        )}
        <Field label="Work operations">{formatCount(trial.work_ops)}</Field>
        <Field label="Elapsed (wall clock)">{formatCount(trial.elapsed_ms)} ms</Field>
        <Field label="Behaviour hash">{trial.behaviour_hash === null ? 'not recorded' : <code>{trial.behaviour_hash}</code>}</Field>
      </dl>
      {schedule?.per_thread_timeslice != null && (
        <details>
          <summary>Per-thread timeslices</summary>
          <pre className="slices">
            <GuestText text={schedule.per_thread_timeslice} max={SLICES_CHARS} expandable={false} />
          </pre>
        </details>
      )}
    </section>
  );
}
