import { absoluteTime, relativeTime } from '../format';

/** An absolute time with the relative time as a second, quieter line. The machine-readable time is in `dateTime`. */
export function TimeStamp({ ms, now, timeZone }: { ms: number; now?: number; timeZone?: string }) {
  return (
    <>
      <time dateTime={new Date(ms).toISOString()}>{absoluteTime(ms, timeZone)}</time>
      <span className="muted time-relative">{relativeTime(ms, now)}</span>
    </>
  );
}
