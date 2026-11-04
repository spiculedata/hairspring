import { renderToString } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { readSource } from '../readSource';
import type { Coverage, CoveragePoint } from '../types';
import { CoverageChart } from './CoverageChart';
import { MAX_CHART_POINTS } from './downsample';

const points = (n: number): CoveragePoint[] =>
  Array.from({ length: n }, (_, i) => ({ seq: i + 1, ids_hit: Math.floor(i / 3), ids_true: Math.floor(i / 5), schedules: i + 1 }));
const coverage = (n: number, over: Partial<Coverage> = {}): Coverage => ({ run_id: 'run_1', trials: n, downsampled: false, points: points(n), ...over });
const text = (html: string) => html.replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' ');

describe('CoverageChart', () => {
  it('draws an SVG with a title and description, and one line per series', () => {
    const html = renderToString(<CoverageChart coverage={coverage(30)} />);
    expect(html.match(/<svg[^>]*role="img"[^>]*aria-labelledby="[^"]+ [^"]+"/g)).toHaveLength(3);
    expect(html).toMatch(/<title id="[^"]+">Property ids reached<\/title>/);
    expect(html.match(/<desc id="[^"]+">/g)).toHaveLength(3);
    expect(html.match(/<polyline/g)).toHaveLength(3);
    expect(html).toMatch(/<figure[^>]*aria-label="Coverage over trials"/);
  });

  it('labels each series directly with its name and final value, and the trial axis with its unit', () => {
    const t = text(renderToString(<CoverageChart coverage={coverage(30)} />));
    expect(t).toContain('Property ids reached');
    expect(t).toContain('Property ids seen true');
    expect(t).toContain('Distinct schedules');
    expect(t).toContain('Trial number');
    expect(t).toMatch(/Distinct schedules[^.]*30/);
  });

  it('states that these are cumulative counts and that a flat line is not a claim about what is left', () => {
    const t = text(renderToString(<CoverageChart coverage={coverage(30)} />));
    expect(t).toMatch(/cumulative/i);
    expect(t).toMatch(/does not mean nothing is left to find/);
  });

  it('gives the same points in a table, behind a disclosure', () => {
    const html = renderToString(<CoverageChart coverage={coverage(12)} />);
    expect(html).toMatch(/<details/);
    expect(html).toContain('Show as a table');
    expect(html.match(/<tbody>.*<\/tbody>/)?.[0].match(/<tr/g)).toHaveLength(12);
  });

  it('does not say downsampled when nothing was thinned', () => {
    expect(text(renderToString(<CoverageChart coverage={coverage(30)} />))).not.toMatch(/downsampled/i);
  });

  it('plots at most 2,000 points, says it was downsampled, and says how many of how many', () => {
    const html = renderToString(<CoverageChart coverage={coverage(5_000)} />);
    const polyline = html.match(/<polyline[^>]*points="([^"]+)"/)?.[1] ?? '';
    expect(polyline.trim().split(/\s+/).length).toBeLessThanOrEqual(MAX_CHART_POINTS);
    expect(text(html)).toContain(`Downsampled: showing ${MAX_CHART_POINTS.toLocaleString('en-GB')} of 5,000 points`);
    expect(html.match(/<tbody>.*<\/tbody>/)?.[0].match(/<tr/g)?.length).toBeLessThanOrEqual(MAX_CHART_POINTS);
  });

  it('says so when the server thinned the series, though every point it sent is drawn', () => {
    const t = text(renderToString(<CoverageChart coverage={coverage(30, { downsampled: true })} />));
    expect(t).toMatch(/Downsampled by the server/);
  });

  it('uses colour tokens only, in the component and its stylesheet', () => {
    for (const f of ['coverage/CoverageChart.tsx', 'coverage/coverage.css']) {
      const src = readSource(f);
      expect(src, f).not.toMatch(/#[0-9a-f]{3,8}\b|rgba?\(/i);
    }
    expect(readSource('coverage/coverage.css')).toMatch(/var\(--/);
  });

  it('never uses a word the product keeps for confirmed findings', () => {
    const t = text(renderToString(<CoverageChart coverage={coverage(5_000)} />));
    expect(t).not.toMatch(/reproduc|discover|found\b/i);
  });
});
