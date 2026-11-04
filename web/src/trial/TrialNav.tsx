import type { ChangeEvent } from 'react';
import type { NavItem } from './navItems';

function scrollToSection(id: string): void {
  document.getElementById(id)?.scrollIntoView?.();
}

function jump(e: ChangeEvent<HTMLSelectElement>): void {
  scrollToSection(e.target.value);
}

/**
 * Jumps to the sections of the page: a list on wide screens and a select on phones, where a list would fill the screen.
 * The list items are buttons, not links, so the address does not change and no link in the page points anywhere but
 * to an app path.
 */
export function TrialNav({ items }: { items: readonly NavItem[] }) {
  return (
    <nav className="trial-nav" aria-label="On this page">
      <ul className="trial-nav-list">
        {items.map((i) => (
          <li key={i.id}>
            <button type="button" className="linklike" data-target={i.id} onClick={() => scrollToSection(i.id)}>
              {i.label}
            </button>
          </li>
        ))}
      </ul>
      <select className="trial-nav-select" aria-label="Jump to section" value="" onChange={jump}>
        <option value="" disabled>
          Jump to section
        </option>
        {items.map((i) => (
          <option key={i.id} value={i.id}>
            {i.label}
          </option>
        ))}
      </select>
    </nav>
  );
}
