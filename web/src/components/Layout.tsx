import type { ReactNode } from 'react';
import { navigate } from '../router';

const LINKS: Array<[string, string]> = [
  ['/', 'Runs'],
  ['/targets', 'Targets'],
  ['/workers', 'Workers'],
];

export function Link({ to, children, current }: { to: string; children: ReactNode; current?: boolean }) {
  return (
    <a
      href={to}
      aria-current={current ? 'page' : undefined}
      onClick={(e) => {
        if (e.metaKey || e.ctrlKey || e.shiftKey || e.button !== 0) return;
        e.preventDefault();
        navigate(to);
      }}
    >
      {children}
    </a>
  );
}

/** Whether the navigation link `to` is the section the current path belongs to. Runs covers a run, its seeds, trials and failures. */
export function isNavActive(to: string, path: string): boolean {
  const within = (base: string): boolean => path === base || path.startsWith(`${base}/`);
  if (to === '/') return path === '/' || within('/runs') || within('/failures');
  return within(to);
}

export function Layout({ path, children, announcement }: { path: string; children: ReactNode; announcement?: string }) {
  return (
    <>
      <a className="skip-link" href="#main">
        Skip to content
      </a>
      <header className="topbar">
        <span className="brand">Hairspring</span>
        <nav aria-label="Main">
          {LINKS.map(([to, label]) => (
            <Link key={to} to={to} current={isNavActive(to, path)}>
              <span className={isNavActive(to, path) ? 'active' : ''}>{label}</span>
            </Link>
          ))}
        </nav>
      </header>
      <div className="sr-only" role="status" aria-live="polite">
        {announcement}
      </div>
      <main id="main" tabIndex={-1}>
        {children}
      </main>
    </>
  );
}
