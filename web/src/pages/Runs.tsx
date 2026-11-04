import { api } from '../api';
import { LoadFailed } from '../components/Notices';
import { StartRunForm } from '../components/StartRunForm';
import { RunsTable } from '../components/RunsTable';
import { UpdatedAt } from '../components/UpdatedAt';
import { startAccess } from '../me';
import { useMe } from '../meStore';
import { navigate } from '../router';
import { usePolling } from '../usePolling';

function NewRun() {
  const targets = usePolling(api.targets, 60_000);
  const access = startAccess(useMe());
  return <StartRunForm targets={targets.data ?? []} start={api.startRun} onStarted={navigate} access={access} />;
}

export function Runs() {
  const runs = usePolling(() => api.runs(50), 2000);
  return (
    <>
      <h1>Runs</h1>
      <NewRun />
      {runs.error && <LoadFailed what="runs" message={runs.error} />}
      <UpdatedAt at={runs.updatedAt} />
      <RunsTable runs={runs.data ?? []} />
    </>
  );
}
