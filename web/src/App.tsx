import { useCallback, useEffect, useState } from 'react';
import { onUnauthorized } from './api';
import { Layout } from './components/Layout';
import { NotFound } from './components/Notices';
import { TokenPrompt } from './components/TokenPrompt';
import { FailurePage } from './pages/FailurePage';
import { Targets, Workers } from './pages/Lists';
import { RunPage } from './pages/RunPage';
import { Runs } from './pages/Runs';
import { MissingPageContext } from './PageStatus';
import { pageTitle } from './pageTitle';
import { matchRoute, usePath, useSearch } from './router';
import { SeedsPage } from './seeds/SeedsPage';
import { SeedPage, TrialPage } from './trial/TrialPage';
import { useDocumentTitle } from './useDocumentTitle';

export function App() {
  const path = usePath();
  const search = useSearch();
  const [needsToken, setNeedsToken] = useState(false);
  const [epoch, setEpoch] = useState(0);
  useEffect(() => onUnauthorized(() => setNeedsToken(true)), []);
  const route = matchRoute(path);
  const validTrial = route.name !== 'trial' || Number.isSafeInteger(Number(route.seq));
  // A page that finds its run, trial or failure missing says so for this path; the title then says the page does not exist.
  const [missingPath, setMissingPath] = useState<string>();
  const reportMissing = useCallback((missing: boolean) => setMissingPath(missing ? window.location.pathname : undefined), []);
  const missing = missingPath === path;
  const title = pageTitle(validTrial && !missing ? route : { name: 'notfound' });
  useDocumentTitle(title);

  if (needsToken) {
    return (
      <Layout path={path} announcement={title}>
        <TokenPrompt
          onDone={() => {
            setNeedsToken(false);
            setEpoch((e) => e + 1);
          }}
        />
      </Layout>
    );
  }

  return (
    <Layout path={path} announcement={title}>
      <MissingPageContext.Provider value={reportMissing}>
      <div key={`${path}${route.name === 'seeds' ? search : ''}:${epoch}`}>
        {route.name === 'runs' && <Runs />}
        {route.name === 'run' && <RunPage id={route.id} />}
        {route.name === 'seeds' && <SeedsPage id={route.id} />}
        {route.name === 'failure' && <FailurePage id={route.id} />}
        {route.name === 'trial' &&
          (validTrial ? <TrialPage runId={route.id} seq={Number(route.seq)} /> : <NotFound what="page" />)}
        {route.name === 'seed' && <SeedPage runId={route.id} seed={route.seed} />}
        {route.name === 'targets' && <Targets />}
        {route.name === 'workers' && <Workers />}
        {route.name === 'notfound' && <NotFound what="page" />}
      </div>
      </MissingPageContext.Provider>
    </Layout>
  );
}
