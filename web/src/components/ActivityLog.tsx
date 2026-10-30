import { describeEvent } from '../activity';
import type { SseEvent } from '../sse';
import { GuestText } from './GuestText';

const VALUE_CHARS = 400;
const RAW_CHARS = 2000;

/**
 * A run's events as readable lines. Values can carry text from a worker, so they are shown through `GuestText`. The raw
 * event stays available behind a disclosure.
 */
export function ActivityLog({ lines }: { lines: readonly SseEvent[] }) {
  if (lines.length === 0) {
    return <p className="muted">No activity yet. Events appear here while the run is going.</p>;
  }
  return (
    <ol className="activity" aria-live="polite">
      {lines.map((line, i) => {
        const { title, facts } = describeEvent(line);
        return (
          <li key={`${line.id}-${i}`}>
            <strong>{title}</strong>
            {facts.map(([label, value]) => (
              <span key={label} className="activity-fact">
                <span className="muted">{label}</span> <GuestText text={value} max={VALUE_CHARS} expandable={false} />
              </span>
            ))}
            <details>
              <summary className="muted">Raw event</summary>
              <pre className="log">
                <GuestText text={`${line.event} ${line.data}`} max={RAW_CHARS} expandable={false} />
              </pre>
            </details>
          </li>
        );
      })}
    </ol>
  );
}
