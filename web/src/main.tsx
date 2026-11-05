import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './App';
import { ConnectionBanner } from './components/ConnectionBanner';
import { applyTheme, readStoredTheme } from './shell/theme';
import './styles.css';
import './components/connection.css';
import './shell/shell.css';
import './explorer/explorer.css';
import './trial/trial.css';
import './seeds/seeds.css';
import './lists/lists.css';
import './workers/workers.css';
import './search/search.css';

applyTheme(readStoredTheme());
const root = document.getElementById('root');
if (!root) throw new Error('missing #root element');
createRoot(root).render(
  <StrictMode>
    <ConnectionBanner />
    <App />
  </StrictMode>,
);
