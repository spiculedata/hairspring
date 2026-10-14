import { stateTone } from '../format';

export function StateBadge({ state }: { state: string }) {
  return <span className={`badge badge-${stateTone(state)}`}>{state}</span>;
}
