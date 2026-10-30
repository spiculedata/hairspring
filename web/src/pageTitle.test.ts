import { describe, expect, it } from 'vitest';
import { pageTitle } from './pageTitle';
import { matchRoute } from './router';

const title = (path: string) => pageTitle(matchRoute(path));

describe('pageTitle', () => {
  it('gives every page its own title ending in the product name', () => {
    const titles = [
      title('/'),
      title('/runs/run_12'),
      title('/runs/run_12/seeds'),
      title('/runs/run_12/trials/7'),
      title('/runs/run_12/seeds/1016'),
      title('/failures/f_9'),
      title('/targets'),
      title('/workers'),
      title('/nowhere'),
    ];
    expect(new Set(titles).size).toBe(titles.length);
    for (const t of titles) expect(t.endsWith(' · Hairspring')).toBe(true);
  });

  it('names the thing the page is about', () => {
    expect(title('/runs/run_12')).toBe('Run run_12 · Hairspring');
    expect(title('/runs/run_12/trials/7')).toBe('Trial 7 of run run_12 · Hairspring');
    expect(title('/runs/run_12/seeds/1016')).toBe('Seed 1016 of run run_12 · Hairspring');
    expect(title('/runs/run_12/seeds')).toBe('Seeds of run run_12 · Hairspring');
    expect(title('/failures/f_9')).toBe('Failure f_9 · Hairspring');
    expect(title('/')).toBe('Runs · Hairspring');
  });

  it('says so when the page does not exist', () => {
    expect(title('/nowhere')).toBe('Page not found · Hairspring');
  });
});
