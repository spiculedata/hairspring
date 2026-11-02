import './confirmation.css';
import {
  CHECKS_HEADING,
  CHECKS_NOTE,
  CHECK_KIND_LABELS,
  ENVIRONMENT_DIFFERS_NOTE,
  ENV_DIFFERS,
  ENV_MATCHES,
  ENV_NOT_REPORTED,
  OTHER_CONFIRMATION_NOTE,
  countsText,
} from '../confirmationWording';
import type { CheckView } from '../types';
import { GuestText } from './GuestText';
import { StatusBadge } from './StatusBadge';
import { TimeStamp } from './TimeStamp';

const WORD_CHARS = 60;
const DETAIL_CHARS = 300;
const KNOWN_STATUSES = ['pass', 'fail', 'inconclusive', 'skipped'];

const own = (table: Readonly<Record<string, string>>, key: string): string | undefined => (Object.hasOwn(table, key) ? table[key] : undefined);

/**
 * Whether a check ran on an environment that differs from the recording, as the runner's detail says. `inconclusive`
 * containing "environment differs" is what `hairspring confirm` looks for; `environment_changed` is the key a bundle
 * replay writes. Neither present means the check did not say.
 */
export function environmentText(detail: Readonly<Record<string, string>>): string {
  if (detail.inconclusive?.includes('environment differs') || detail.environment_changed === 'true') return ENV_DIFFERS;
  return detail.environment_changed === 'false' ? ENV_MATCHES : ENV_NOT_REPORTED;
}

function CheckRow({ check, inFinding }: { check: CheckView; inFinding: boolean }) {
  const kind = own(CHECK_KIND_LABELS, check.kind);
  const known = KNOWN_STATUSES.includes(check.status);
  const detail = Object.entries(check.guest.detail);
  return (
    <tr>
      <td className="num">{check.ordinal}</td>
      <td>{kind ?? <GuestText text={check.kind} max={WORD_CHARS} expandable={false} />}</td>
      <td>
        <StatusBadge kind="check" status={check.status} />
        {!known && (
          <>
            {' '}
            <GuestText text={check.status} max={WORD_CHARS} expandable={false} />
          </>
        )}
      </td>
      <td>{countsText(check.counts)}</td>
      <td>{environmentText(check.guest.detail)}</td>
      <td>
        <TimeStamp ms={check.started_at} />
      </td>
      <td>{check.runner_trusted ? 'Yes' : 'No'}</td>
      <td>{inFinding ? 'Yes' : 'No'}</td>
      <td>
        <details>
          <summary>Details</summary>
          <p>
            Command: <code>{<GuestText text={check.guest.command} max={DETAIL_CHARS} />}</code>
          </p>
          {detail.length === 0 ? (
            <p className="muted">No notes were reported.</p>
          ) : (
            <dl className="check-detail">
              {detail.map(([k, v]) => (
                <div key={k}>
                  <dt>
                    <GuestText text={k} max={WORD_CHARS} expandable={false} />
                  </dt>
                  <dd>
                    <GuestText text={v} max={DETAIL_CHARS} />
                  </dd>
                </div>
              ))}
            </dl>
          )}
        </details>
      </td>
    </tr>
  );
}

/**
 * Every stored check of a failure, in the order the server sends them, with its result, counts as recorded, and what
 * the runner said about the environment. Free text from the runner goes through `GuestText`. A check from a
 * confirmation other than the one the finding comes from is listed and marked as not counting towards it.
 */
export function CheckTable({ checks, confirmationId }: { checks: readonly CheckView[]; confirmationId: string | null }) {
  const anyOther = checks.some((c) => c.confirmation_id !== confirmationId);
  const differs = checks.some((c) => c.status === 'inconclusive' && environmentText(c.guest.detail) === ENV_DIFFERS);
  return (
    <section aria-labelledby="checks-heading">
      <h2 id="checks-heading">{CHECKS_HEADING}</h2>
      <table className="checks">
        <thead>
          <tr>
            <th className="num">Order</th>
            <th>Check</th>
            <th>Result</th>
            <th>Runs counted</th>
            <th>Environment</th>
            <th>Started</th>
            <th>Trusted runner</th>
            <th>Counts towards the finding</th>
            <th>Details</th>
          </tr>
        </thead>
        <tbody>
          {checks.map((c) => (
            <CheckRow key={c.id} check={c} inFinding={c.confirmation_id === confirmationId} />
          ))}
        </tbody>
      </table>
      {differs && <p role="note">{ENVIRONMENT_DIFFERS_NOTE}</p>}
      {anyOther && <p className="muted">{OTHER_CONFIRMATION_NOTE}</p>}
      <p className="muted">{CHECKS_NOTE}</p>
    </section>
  );
}
