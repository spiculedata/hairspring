import type { ReactNode } from 'react';
import { navigate } from '../router';
import { SearchBox } from '../search/SearchBox';
import { ThemeSwitch } from '../shell/ThemeSwitch';

const LINKS: Array<[string, string]> = [
  ['/', 'Overview'],
  ['/runs', 'Runs'],
  ['/failures', 'Failures'],
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

/** Whether the navigation link `to` is the section the current path belongs to. Overview is the home address only. Runs covers the list, a run, its seeds, trials and one failure's page; Failures is the list. */
export function isNavActive(to: string, path: string): boolean {
  const within = (base: string): boolean => path === base || path.startsWith(`${base}/`);
  if (to === '/') return path === '/';
  if (to === '/runs') return within('/runs') || path.startsWith('/failures/');
  if (to === '/failures') return path === '/failures';
  return within(to);
}

export function Layout({
  path,
  children,
  announcement,
  organisation,
}: {
  path: string;
  children: ReactNode;
  announcement?: string;
  /** The organisation's name from the server's configuration. Shown in the top bar when there is one, and nowhere otherwise. */
  organisation?: string;
}) {
  return (
    <>
      <a className="skip-link" href="#main">
        Skip to content
      </a>
      <header className="topbar">
        <div className="topbar-id">
          <span className="brand">Hairspring</span>
          {organisation !== undefined && organisation !== '' && <span className="org-name">{organisation}</span>}
        </div>
        <SearchBox />
        <ThemeSwitch />
      </header>
      <div className="sr-only" role="status" aria-live="polite">
        {announcement}
      </div>
      <div className="frame">
        <nav className="sidenav" aria-label="Main">
          <ul>
            {LINKS.map(([to, label]) => (
              <li key={to}>
                <Link to={to} current={isNavActive(to, path)}>
                  {label}
                </Link>
              </li>
            ))}
          </ul>
        </nav>
        <main id="main" tabIndex={-1}>
          {children}
        </main>
      </div>
    </>
  );
}
