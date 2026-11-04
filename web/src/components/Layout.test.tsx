import { renderToString } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { readSource } from '../readSource';
import { Layout, isNavActive } from './Layout';

const render = (path: string) => renderToString(<Layout path={path}>body</Layout>);

describe('Layout', () => {
  it('starts with a skip link to the main content, and the main content can take focus', () => {
    const html = render('/');
    expect(html.indexOf('class="skip-link"')).toBeGreaterThan(-1);
    expect(html.indexOf('class="skip-link"')).toBeLessThan(html.indexOf('<header'));
    expect(html).toContain('href="#main"');
    expect(html).toMatch(/<main id="main" tabindex="-1"/);
    expect(html).toContain('Skip to content');
  });

  it('puts the navigation in a side column beside the main content, with the theme switch in the top bar', () => {
    const html = render('/');
    expect(html).toMatch(/<header class="topbar">[\s\S]*role="group"[\s\S]*<\/header>/);
    expect(html).toMatch(/<div class="frame"><nav class="sidenav" aria-label="Main">/);
    expect(html.indexOf('class="sidenav"')).toBeLessThan(html.indexOf('<main'));
    expect(html.match(/<nav /g)).toHaveLength(1);
  });

  it('collapses the side column into a wrapping top bar on phones, with no fixed widths that could scroll sideways', () => {
    const shell = readSource('shell/shell.css');
    const phone = shell.slice(shell.indexOf('@media (max-width: 860px)'));
    expect(phone).toMatch(/\.frame \{[^}]*grid-template-columns: minmax\(0, 1fr\)/);
    expect(phone).toMatch(/\.sidenav ul \{[^}]*flex-wrap: wrap/);
    expect(shell).toMatch(/\.frame > main \{[^}]*min-width: 0/);
    expect(shell).not.toMatch(/(?<!min-|max-)width:\s*\d+(px|rem)/);
  });

  it('shows no organisation name in the top bar unless one is set', () => {
    expect(render('/')).not.toContain('org-name');
    const empty = renderToString(<Layout path="/" organisation="">body</Layout>);
    expect(empty).not.toContain('org-name');
  });

  it('shows the organisation name beside the brand, as text, when one is set', () => {
    const html = renderToString(<Layout path="/" organisation="Acme Ltd">body</Layout>);
    expect(html).toMatch(
      /<header class="topbar"><div class="topbar-id"><span class="brand">Hairspring<\/span><span class="org-name">Acme Ltd<\/span><\/div>/,
    );
    expect(html.match(/Acme Ltd/g)).toHaveLength(1);
  });

  it('escapes markup in the organisation name', () => {
    const html = renderToString(<Layout path="/" organisation={'<img src=x onerror=alert(1)>'}>body</Layout>);
    expect(html).not.toContain('<img');
    expect(html).toContain('&lt;img src=x onerror=alert(1)&gt;');
  });

  it('lets a long organisation name wrap instead of widening the page', () => {
    const shell = readSource('shell/shell.css');
    expect(shell).toMatch(/\.org-name \{[^}]*overflow-wrap: anywhere/);
    expect(shell).toMatch(/\.org-name \{[^}]*min-width: 0/);
  });

  it('marks the current page in the navigation with aria-current, on the link, and only there', () => {
    const html = render('/workers');
    expect(html.match(/aria-current="page"/g)).toHaveLength(1);
    expect(html).toMatch(/<a href="\/workers" aria-current="page"/);
    expect(html).not.toMatch(/<a href="\/targets" aria-current/);
  });

  it('has a Failures link, marked current on the list only, and the nav order is Overview, Runs, Failures, Targets, Workers', () => {
    const html = render('/failures');
    expect(html).toMatch(/<a href="\/failures" aria-current="page"/);
    expect(html.match(/aria-current="page"/g)).toHaveLength(1);
    const order = ['href="/"', 'href="/runs"', 'href="/failures"', 'href="/targets"', 'href="/workers"'].map((s) => html.indexOf(s));
    expect([...order].sort((a, b) => a - b)).toEqual(order);
    expect(isNavActive('/failures', '/failures')).toBe(true);
    expect(isNavActive('/runs', '/failures')).toBe(false);
    expect(isNavActive('/failures', '/failures/f_1')).toBe(false);
  });

  it('counts a run, its seeds, its trials and a failure as part of Runs', () => {
    for (const path of ['/runs', '/runs/run_1', '/runs/run_1/seeds', '/runs/run_1/trials/3', '/failures/f_1']) {
      expect(isNavActive('/runs', path), path).toBe(true);
      expect(isNavActive('/', path), path).toBe(false);
      expect(isNavActive('/targets', path), path).toBe(false);
    }
    expect(isNavActive('/targets', '/targets')).toBe(true);
    expect(isNavActive('/runs', '/targets')).toBe(false);
    expect(isNavActive('/runs', '/nowhere')).toBe(false);
    // Whole path segments only: /runsx is not a run page.
    expect(isNavActive('/runs', '/runsx')).toBe(false);
    expect(isNavActive('/runs', '/failuresx/1')).toBe(false);
    expect(isNavActive('/targets', '/targetsx')).toBe(false);
  });

  it('marks Overview current at the home address only, and Runs is not current there', () => {
    const html = render('/');
    expect(html).toMatch(/<a href="\/" aria-current="page"/);
    expect(html.match(/aria-current="page"/g)).toHaveLength(1);
    expect(isNavActive('/', '/')).toBe(true);
    expect(isNavActive('/', '/runs')).toBe(false);
    expect(isNavActive('/', '/targets')).toBe(false);
  });
});
