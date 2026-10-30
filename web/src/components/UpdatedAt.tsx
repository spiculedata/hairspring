import { timeOfDay } from '../format';

/** When the data on a page was last loaded, so a reader can tell a page that has gone stale. Nothing until there is data. */
export function UpdatedAt({ at, timeZone }: { at: number | undefined; timeZone?: string }) {
  if (at === undefined) return null;
  return <p className="muted updated-at">{`Updated ${timeOfDay(at, timeZone)}`}</p>;
}
