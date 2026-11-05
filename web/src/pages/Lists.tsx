import { api } from '../api';
import { LoadFailed } from '../components/Notices';
import { ScrollRegion } from '../components/ScrollRegion';
import { UpdatedAt } from '../components/UpdatedAt';
import { usePolling } from '../usePolling';

const TABLE_MAX_HEIGHT = 640;
const SCROLL_HINT = 'Scroll to see more of the table.';

export function Targets() {
  const t = usePolling(api.targets, 60_000);
  return (
    <>
      <h1>Targets</h1>
      {t.error && <LoadFailed what="targets" message={t.error} network={t.networkError} />}
      <UpdatedAt at={t.updatedAt} />
      <ScrollRegion label="Targets" hint={SCROLL_HINT} maxHeight={TABLE_MAX_HEIGHT}>
        <table>
          <thead>
            <tr><th>Name</th><th>Kind</th><th>Budget</th><th>Description</th></tr>
          </thead>
          <tbody>
            {(t.data ?? []).map((x) => (
              <tr key={x.name}>
                <td>{x.name}</td>
                <td>{x.kind}</td>
                <td className="muted">{x.budget === undefined ? '' : `${x.budget.max_steps.toLocaleString()} steps / ${x.budget.wall_secs}s`}</td>
                <td>{x.description}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </ScrollRegion>
    </>
  );
}
