import type { ReactNode } from 'react';
import { GuestText } from '../components/GuestText';
import { formatCount, NO_SCHEDULE_NOTE } from '../wording';
import { parseThreadSlices, parseTimesliceStats, type ThreadSlices } from './schedule';
import type { ScheduleData, TrialRow } from './types';

const FIELD_CHARS = 400;
const NAME_CHARS = 80;
const SLICES_CHARS = 8 * 1024;

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <>
      <dt>{label}</dt>
      <dd>{children}</dd>
    </>
  );
}

/** The timeslice count and total as two fields when the text is in the expected shape, else the text as reported. */
function TimesliceFields({ text }: { text: string }) {
  const stats = parseTimesliceStats(text);
  if (stats === undefined) {
    return (
      <Field label="Timeslice statistics">
        <GuestText text={text} max={FIELD_CHARS} expandable={false} />
      </Field>
    );
  }
  return (
    <>
      <Field label="Timeslices">{formatCount(stats.count)}</Field>
      <Field label="Timeslice total">{formatCount(stats.sumNs)} ns</Field>
    </>
  );
}

function SliceRows({ parsed }: { parsed: ThreadSlices }) {
  return (
    <div className="table-scroll" role="region" aria-label="Per-thread timeslices, scrollable" tabIndex={0}>
      <table className="slices-table" aria-label="Per-thread timeslices">
        <thead>
          <tr>
            {parsed.columns.map((c) => (
              <th key={c} scope="col">
                <GuestText text={c} max={NAME_CHARS} expandable={false} />
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {parsed.rows.map((row, i) => (
            <tr key={i}>
              {row.map((cell, j) => (
                <td key={j}>
                  <GuestText text={cell} max={NAME_CHARS} expandable={false} />
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/** Per-thread timeslices as a table when they parse as one, else the text as reported. */
function SliceTable({ text }: { text: string }) {
  const parsed = parseThreadSlices(text);
  if (parsed !== undefined) return <SliceRows parsed={parsed} />;
  return (
    <pre className="slices">
      <GuestText text={text} max={SLICES_CHARS} expandable={false} />
    </pre>
  );
}

/**
 * The scheduler summary's deterministic fields (turns, syscalls, virtual time, timeslices), the schedule fingerprint and
 * the row's own figures. The fields are as the worker reported them, not checked against Hermit; none is wall-clock time.
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
        {schedule?.timeslice_stats != null && <TimesliceFields text={schedule.timeslice_stats} />}
        <Field label="Work operations">{formatCount(trial.work_ops)}</Field>
        <Field label="Elapsed (wall clock)">{formatCount(trial.elapsed_ms)} ms</Field>
        <Field label="Behaviour hash">{trial.behaviour_hash === null ? 'not recorded' : <code>{trial.behaviour_hash}</code>}</Field>
      </dl>
      {schedule?.per_thread_timeslice != null && (
        <details>
          <summary>Per-thread timeslices</summary>
          <SliceTable text={schedule.per_thread_timeslice} />
        </details>
      )}
    </section>
  );
}
