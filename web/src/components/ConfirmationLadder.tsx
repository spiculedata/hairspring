import './confirmation.css';
import {
  CURRENT_MARK,
  ENVIRONMENT_NOT_PINNED,
  ENVIRONMENT_PINNED,
  END_NOTE,
  FINDING_LABELS,
  FINDING_MEANINGS,
  LADDER_ENDS,
  LADDER_HEADING,
  LADDER_NOTE,
  LADDER_RUNGS,
  NOT_REACHED_MARK,
  REACHED_MARK,
  UNKNOWN_FINDING_TEXT,
} from '../confirmationWording';
import { Term } from '../glossary/Term';
import { GuestText } from './GuestText';
import { StatusBadge } from './StatusBadge';

const has = (list: readonly string[], word: string): boolean => list.includes(word);

function markOf(index: number, current: number): string {
  if (index === current) return CURRENT_MARK;
  return index < current ? REACHED_MARK : NOT_REACHED_MARK;
}

/**
 * Where a failure stands, from the state the server derived from its stored checks. The four rungs are shown in the
 * server's order with the current one marked in words; the two ends (`not_reproducible`, `control_failed`) are not
 * rungs, so no rung is marked for them. A word the page has no wording for is shown as sent, with no position.
 * "Confirmed" and "verified" appear only when the server says so.
 */
export function ConfirmationLadder({ finding, environmentPinned }: { finding: string; environmentPinned: boolean }) {
  const current = LADDER_RUNGS.findIndex((r) => r === finding);
  const isEnd = has(LADDER_ENDS, finding);
  const known = current >= 0 || isEnd;
  const showsEnvironment = finding === 'confirmed' || finding === 'verified';
  return (
    <section className="ladder-section" aria-labelledby="confirmation-heading">
      <h2 id="confirmation-heading">
        <Term term="confirmation">{LADDER_HEADING}</Term>
      </h2>
      <p>
        <StatusBadge kind="finding" status={finding} />
        {showsEnvironment && <span className="muted ladder-env">{` ${environmentPinned ? ENVIRONMENT_PINNED : ENVIRONMENT_NOT_PINNED}`}</span>}
      </p>
      {known ? (
        <p>{FINDING_MEANINGS[finding]}</p>
      ) : (
        <p>
          {UNKNOWN_FINDING_TEXT} <GuestText text={finding} max={80} expandable={false} />
        </p>
      )}
      {isEnd && <p className="muted">{END_NOTE}</p>}
      <ol className="ladder">
        {LADDER_RUNGS.map((rung, i) => (
          <li key={rung} className={i === current ? 'ladder-current' : undefined} aria-current={i === current ? 'step' : undefined}>
            <strong>{FINDING_LABELS[rung]}</strong>
            {current >= 0 && <span className="ladder-mark">{markOf(i, current)}</span>}
            <span className="muted">{FINDING_MEANINGS[rung]}</span>
          </li>
        ))}
      </ol>
      <p className="muted">{LADDER_NOTE}</p>
    </section>
  );
}
