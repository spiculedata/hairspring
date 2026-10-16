import { useEffect, useState } from 'react';
import { onUnauthorized } from './api';
import { Layout } from './components/Layout';
import { TokenPrompt } from './components/TokenPrompt';
import { FailurePage } from './pages/FailurePage';
import { Targets, Workers } from './pages/Lists';
import { RunPage } from './pages/RunPage';
import { Runs } from './pages/Runs';
import { matchRoute, usePath, useSearch } from './router';
import { SeedsPage } from './seeds/SeedsPage';
import { SeedPage, TrialPage } from './trial/TrialPage';

export function App() {
  const path = usePath();
  const search = useSearch();
  const [needsToken, setNeedsToken] = useState(false);
  const [epoch, setEpoch] = useState(0);
  useEffect(() => onUnauthorized(() => setNeedsToken(true)), []);

  if (needsToken) {
    return (
      <Layout path={path}>
        <TokenPrompt
          onDone={() => {
            setNeedsToken(false);
            setEpoch((e) => e + 1);
          }}
        />
      </Layout>
    );
  }

  const route = matchRoute(path);
  return (
    <Layout path={path}>
      <div key={`${path}${route.name === 'seeds' ? search : ''}:${epoch}`}>
        {route.name === 'runs' && <Runs />}
        {route.name === 'run' && <RunPage id={route.id} />}
        {route.name === 'seeds' && <SeedsPage id={route.id} />}
        {route.name === 'failure' && <FailurePage id={route.id} />}
        {route.name === 'trial' &&
          (Number.isSafeInteger(Number(route.seq)) ? <TrialPage runId={route.id} seq={Number(route.seq)} /> : <p className="muted">Page not found.</p>)}
        {route.name === 'seed' && <SeedPage runId={route.id} seed={route.seed} />}
        {route.name === 'targets' && <Targets />}
        {route.name === 'workers' && <Workers />}
        {route.name === 'notfound' && <p className="muted">Page not found.</p>}
      </div>
    </Layout>
  );
}
