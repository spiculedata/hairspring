import { useEffect, useState } from 'react';

const NAV_EVENT = 'hairspring:navigate';

export function navigate(path: string): void {
  if (window.location.pathname !== path) window.history.pushState({}, '', path);
  window.dispatchEvent(new Event(NAV_EVENT));
}

export function usePath(): string {
  const [path, setPath] = useState(window.location.pathname);
  useEffect(() => {
    const update = () => setPath(window.location.pathname);
    window.addEventListener('popstate', update);
    window.addEventListener(NAV_EVENT, update);
    return () => {
      window.removeEventListener('popstate', update);
      window.removeEventListener(NAV_EVENT, update);
    };
  }, []);
  return path;
}

export type Route =
  | { name: 'runs' }
  | { name: 'run'; id: string }
  | { name: 'failure'; id: string }
  | { name: 'targets' }
  | { name: 'workers' }
  | { name: 'notfound' };

export function matchRoute(path: string): Route {
  const parts = path.split('/').filter(Boolean);
  if (parts.length === 0 || (parts.length === 1 && parts[0] === 'runs')) return { name: 'runs' };
  if (parts.length === 2 && parts[0] === 'runs') return { name: 'run', id: parts[1] };
  if (parts.length === 2 && parts[0] === 'failures') return { name: 'failure', id: parts[1] };
  if (parts.length === 1 && parts[0] === 'targets') return { name: 'targets' };
  if (parts.length === 1 && parts[0] === 'workers') return { name: 'workers' };
  return { name: 'notfound' };
}
