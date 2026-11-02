import { renderToString } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
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
