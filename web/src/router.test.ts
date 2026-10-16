import { describe, expect, it } from 'vitest';
import { matchRoute, seedPath, trialPath } from './router';

describe('trial routes', () => {
  it('matches a trial by sequence number and a seed by its decimal string', () => {
    expect(matchRoute('/runs/run_0123/trials/16')).toEqual({ name: 'trial', id: 'run_0123', seq: '16' });
    expect(matchRoute('/runs/run_0123/seeds/18446744073709551615')).toEqual({ name: 'seed', id: 'run_0123', seed: '18446744073709551615' });
  });

  it('keeps the existing routes', () => {
    expect(matchRoute('/runs/run_0123')).toEqual({ name: 'run', id: 'run_0123' });
    expect(matchRoute('/failures/f1')).toEqual({ name: 'failure', id: 'f1' });
    expect(matchRoute('/')).toEqual({ name: 'runs' });
  });

  it('is not found for a number that is not plain digits, or one that is too long to be a seed', () => {
    for (const p of ['/runs/r/trials/x', '/runs/r/trials/-1', '/runs/r/seeds/1e3', '/runs/r/trials/16/extra', `/runs/r/seeds/${'9'.repeat(21)}`]) {
      expect(matchRoute(p).name, p).toBe('notfound');
    }
  });
});

describe('the seed explorer route', () => {
  it('is a run id followed by seeds, with nothing after it', () => {
    expect(matchRoute('/runs/run_0123/seeds')).toEqual({ name: 'seeds', id: 'run_0123' });
    expect(matchRoute('/runs/run_0123/seeds/')).toEqual({ name: 'seeds', id: 'run_0123' });
  });

  it('checks the run id like the other run routes', () => {
    for (const p of ['/runs/a%20b/seeds', '/runs/a.b/seeds', `/runs/${'a'.repeat(65)}/seeds`, '/runs/run_0123/seed']) {
      expect(matchRoute(p).name, p).toBe('notfound');
    }
  });
});

describe('run ids on trial routes', () => {
  it('are checked like the paths builders check them', () => {
    for (const p of ['/runs/a%20b/trials/1', '/runs/a.b/seeds/1', `/runs/${'a'.repeat(65)}/trials/1`]) {
      expect(matchRoute(p).name, p).toBe('notfound');
    }
  });
});

describe('paths', () => {
  it('builds paths from validated pieces and refuses anything else', () => {
    expect(seedPath('run_0123', '1016')).toBe('/runs/run_0123/seeds/1016');
    expect(trialPath('run_0123', 16)).toBe('/runs/run_0123/trials/16');
    expect(seedPath('run_0123', '../x')).toBeUndefined();
    expect(seedPath('run 1', '1')).toBeUndefined();
    expect(trialPath('run_0123', -1)).toBeUndefined();
    expect(trialPath('run_0123', 1.5)).toBeUndefined();
  });
});
