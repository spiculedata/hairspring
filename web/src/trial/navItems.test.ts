import { describe, expect, it } from 'vitest';
import { navItems } from './navItems';

describe('navItems', () => {
  it('lists every section when detail was kept, in page order', () => {
    expect(navItems(true).map((i) => i.id)).toEqual(['witness-title', 'threads-title', 'procs-title', 'health-title', 'schedule-title', 'output-title', 'about-title']);
  });

  it('leaves out the sections that need kept detail when none was kept', () => {
    expect(navItems(false).map((i) => i.id)).toEqual(['schedule-title', 'output-title', 'about-title']);
  });
});
