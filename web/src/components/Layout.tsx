import type { ReactNode } from 'react';
import { navigate } from '../router';

const LINKS: Array<[string, string]> = [
  ['/', 'Runs'],
  ['/targets', 'Targets'],
  ['/workers', 'Workers'],
];

export function Link({ to, children }: { to: string; children: ReactNode }) {
  return (
    <a
      href={to}
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

export function Layout({ path, children }: { path: string; children: ReactNode }) {
  return (
    <>
      <header className="topbar">
        <span className="brand">Hairspring</span>
        <nav aria-label="Main">
          {LINKS.map(([to, label]) => (
            <Link key={to} to={to}>
              <span className={(to === '/' ? path === '/' || path.startsWith('/runs') : path.startsWith(to)) ? 'active' : ''}>{label}</span>
            </Link>
          ))}
        </nav>
      </header>
      <main>{children}</main>
    </>
  );
}
