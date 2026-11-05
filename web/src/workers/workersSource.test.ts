import { describe, expect, it } from 'vitest';
import { readSource } from '../readSource';
import * as wording from './workersWording';

const FILES = ['workers/WorkersPage.tsx', 'workers/workersModel.ts', 'workers/workersWording.ts', 'workers/workers.css'];

describe('the Workers page source', () => {
  it('polls through the shared loop and never starts a timer of its own', () => {
    const src = readSource('workers/WorkersPage.tsx');
    expect(src).toMatch(/usePolling\(/);
    expect(src).not.toMatch(/setInterval|setTimeout/);
  });

  it('writes no colour literal and uses no forbidden word, hype word or US spelling', () => {
    for (const file of FILES) {
      const src = readSource(file);
      expect(src, file).not.toMatch(/#[0-9a-fA-F]{3,8}\b|\brgba?\(|\bhsla?\(/);
      expect(src, file).not.toMatch(/\b(color|behavior|authorize|initialize|organize|favor|center)\w*/i);
      expect(src, file).not.toMatch(/\b(powerful|seamless|robust|blazing|effortless|cutting-edge|revolutionary)\b/i);
    }
  });

  it('shows no emoji and never says found, discovered or unassisted in what it says', () => {
    for (const [name, value] of Object.entries(wording)) {
      if (typeof value !== 'string') continue;
      expect(value, name).not.toMatch(/\p{Extended_Pictographic}|\bfound\b|\b(discovered|unassisted|flaky)\b/iu);
    }
  });

  it('never puts a worker id, secret or token anywhere but the id cell, and renders nothing as markup', () => {
    const src = readSource('workers/WorkersPage.tsx');
    expect(src).not.toMatch(/dangerouslySetInnerHTML|innerHTML/);
    expect(src).not.toMatch(/secret|token|digest|payload/i);
    expect(src.match(/worker\.id/g)?.length).toBe(3);
  });

  it('keeps the list cells breakable and the table in a scroll region, so 320 px does not scroll the page', () => {
    const css = readSource('workers/workers.css');
    expect(css).toMatch(/\.worker-list li \{[^}]*overflow-wrap: anywhere/);
    expect(readSource('workers/WorkersPage.tsx')).toMatch(/<ScrollRegion[^>]*>\s*<table className="workers">/);
  });
});
