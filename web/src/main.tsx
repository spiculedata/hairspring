import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './App';
import { ConnectionBanner } from './components/ConnectionBanner';
import './styles.css';
import './components/connection.css';
import './explorer/explorer.css';
import './trial/trial.css';
import './seeds/seeds.css';

const root = document.getElementById('root');
if (!root) throw new Error('missing #root element');
createRoot(root).render(
  <StrictMode>
    <ConnectionBanner />
    <App />
  </StrictMode>,
);
