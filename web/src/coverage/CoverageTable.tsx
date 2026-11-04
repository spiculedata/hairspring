import { ScrollRegion } from '../components/ScrollRegion';
import type { CoveragePoint } from '../types';
import { formatCount, PANEL_KEYS } from './coverageModel';
import { AXIS_LABEL, PANEL_LABELS, SHOW_TABLE } from './coverageWording';

const TABLE_MAX_HEIGHT = 320;

/** The chart's points as a table, for a reader who cannot see the lines or wants the exact figures. */
export function CoverageTable({ points }: { points: readonly CoveragePoint[] }) {
  return (
    <details className="coverage-table">
      <summary>{SHOW_TABLE}</summary>
      <ScrollRegion label="Coverage points" hint="Scroll to see more of the table." maxHeight={TABLE_MAX_HEIGHT}>
        <table>
          <caption className="coverage-sr">Coverage points, one row per point drawn</caption>
          <thead>
            <tr>
              <th scope="col">{AXIS_LABEL}</th>
              {PANEL_KEYS.map((k) => (
                <th key={k} scope="col">
                  {PANEL_LABELS[k]}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {points.map((p, i) => (
              <tr key={`${i}-${p.seq}`}>
                <th scope="row">{formatCount(p.seq)}</th>
                {PANEL_KEYS.map((k) => (
                  <td key={k} className="coverage-num">
                    {formatCount(p[k])}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </ScrollRegion>
    </details>
  );
}
