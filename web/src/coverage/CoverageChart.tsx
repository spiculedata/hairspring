import { useId, useMemo } from 'react';
import type { Coverage } from '../types';
import { buildPanels, formatCount, panelSummary, type Panel, type PanelKey } from './coverageModel';
import { CoverageTable } from './CoverageTable';
import { AXIS_LABEL, COVERAGE_NOTE, COVERAGE_TITLE, downsampledNote, PANEL_LABELS } from './coverageWording';
import { downsamplePoints } from './downsample';
import './coverage.css';

const VIEW = { width: 600, height: 80 };

const pointsAttr = (panel: Panel): string => panel.xs.map((x, i) => `${x.toFixed(1)},${(panel.ys[i] ?? 0).toFixed(1)}`).join(' ');

interface PanelViewProps {
  panel: Panel;
  summary: string;
}

/** One series on its own scale. Text is HTML, so it stays readable on a narrow screen; the line is SVG. */
function PanelView({ panel, summary }: PanelViewProps) {
  const titleId = useId();
  const descId = useId();
  const label = PANEL_LABELS[panel.key];
  return (
    <div className="coverage-panel" data-series={panel.key}>
      <h3>{label}</h3>
      <p className="muted coverage-summary">{summary}</p>
      <div className="coverage-plot">
        <div className="coverage-yaxis" aria-hidden="true">
          <span>{formatCount(panel.top)}</span>
          <span>0</span>
        </div>
        <svg viewBox={`0 0 ${VIEW.width} ${VIEW.height}`} preserveAspectRatio="none" role="img" aria-labelledby={`${titleId} ${descId}`}>
          <title id={titleId}>{label}</title>
          <desc id={descId}>{`${label}, cumulative by trial number: ${summary}. The same figures are in the table.`}</desc>
          <line className="coverage-grid" x1="0" y1={VIEW.height} x2={VIEW.width} y2={VIEW.height} />
          <line className="coverage-grid" x1="0" y1="0" x2={VIEW.width} y2="0" />
          <polyline className={`coverage-line coverage-line-${panel.key}`} fill="none" points={pointsAttr(panel)} />
        </svg>
      </div>
    </div>
  );
}

/** The run's coverage as one small chart per series, a shared trial axis, direct labels and a table alternative. */
export function CoverageChart({ coverage }: { coverage: Coverage }) {
  const shown = useMemo(() => downsamplePoints(coverage.points), [coverage.points]);
  const panels = useMemo(() => buildPanels(shown.points, VIEW), [shown.points]);
  const note = downsampledNote(shown.points.length, coverage.points.length, coverage.downsampled);
  const first = shown.points[0]?.seq ?? 0;
  const last = shown.points[shown.points.length - 1]?.seq ?? 0;
  return (
    <figure className="coverage-chart" aria-label={COVERAGE_TITLE}>
      <p className="muted">{COVERAGE_NOTE}</p>
      {note && <p className="coverage-downsampled">{note}</p>}
      {panels.map((panel) => (
        <PanelView key={panel.key} panel={panel} summary={panelSummary(panel.key as PanelKey, coverage.points)} />
      ))}
      <p className="coverage-xaxis muted" aria-hidden="true">
        <span>{formatCount(first)}</span>
        <span>{AXIS_LABEL}</span>
        <span>{formatCount(last)}</span>
      </p>
      <CoverageTable points={shown.points} />
    </figure>
  );
}
