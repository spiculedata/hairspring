import { TierChips } from '../components/TierChips';
import type { PropertyPage } from '../types';
import { excludedText, noClaimSentence, runCounts, workersText } from '../wording';

/**
 * The top of the overview: the generated headline, label chips, the claim sentence (only when the server made one),
 * how the run ended and what was left out of every rate. All wording is the server's or wording.ts.
 */
export function RunSummary({ page }: { page: PropertyPage }) {
  const { header, claim } = page;
  const excluded = excludedText(header.excluded);
  return (
    <div className="card run-summary">
      <p className="headline">
        <strong>{page.headline}</strong>
      </p>
      <TierChips chips={claim.chips} caveats={claim.caveats} unverified={header.unverified_worker_data} />
      <p className={claim.text ? 'claim' : 'muted'}>{claim.text ?? noClaimSentence(claim.withheld)}</p>
      <p className="muted">{runCounts(header)}</p>
      <p className="muted">{workersText(header.workers_contributing, header.workers_untrusted)}</p>
      {excluded && <p className="excluded">{excluded}</p>}
      {page.notes.length > 0 && (
        <ul className="notes">
          {page.notes.map((n) => (
            <li key={n}>{n}</li>
          ))}
        </ul>
      )}
    </div>
  );
}
