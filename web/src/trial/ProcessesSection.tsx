import { GuestText } from '../components/GuestText';
import { formatCount } from '../wording';
import { healthCounters, processState } from './model';
import type { KeptHealth, KeptProc } from './types';

const NAME_CHARS = 80;
const MARKS = { ok: '✓', ended: '△', missing: '?' } as const;

function Cell({ text }: { text: string | null }) {
  return text === null ? <>–</> : <GuestText text={text} max={NAME_CHARS} expandable={false} />;
}

/** One row per process: who reported, what it ended with, and how much work it did. */
export function ProcessesSection({ procs }: { procs: readonly KeptProc[] }) {
  return (
    <section aria-labelledby="procs-title">
      <h2 id="procs-title">Processes</h2>
      <div className="table-scroll" role="region" aria-label="Processes, scrollable" tabIndex={0}>
        <table className="procs" aria-label="Processes">
          <thead>
            <tr>
              <th scope="col">Process</th>
              <th scope="col">SDK</th>
              <th scope="col">Catalog</th>
              <th scope="col">Threads</th>
              <th scope="col">Work</th>
              <th scope="col">End</th>
            </tr>
          </thead>
          <tbody>
            {procs.map((p, i) => {
              const state = processState(p);
              return (
                <tr key={i}>
                  <th scope="row">
                    <Cell text={p.tag} />
                  </th>
                  <td>
                    <Cell text={p.sdk} /> <Cell text={p.sdk_version} />
                  </td>
                  <td>
                    <Cell text={p.catalog_hash} />
                  </td>
                  <td className="num">{formatCount(p.threads.length)}</td>
                  <td className="num">{p.work_ops === null ? '–' : formatCount(p.work_ops)}</td>
                  <td>
                    <span aria-hidden="true">{MARKS[state.kind]}</span> {state.text}
                    {p.end !== null && (
                      <span className="muted">
                        {' '}
                        · {formatCount(p.end.evals)} evals, {formatCount(p.end.sites)} sites, {formatCount(p.end.dropped)} dropped
                      </span>
                    )}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </section>
  );
}

/** Event health in words: how many lines were accepted, whether the trial counts as degraded, and what was lost. */
export function HealthSection({ health, degraded }: { health: KeptHealth; degraded: boolean }) {
  const { accepted, problems } = healthCounters(health);
  return (
    <section aria-labelledby="health-title">
      <h2 id="health-title">Event health</h2>
      <p>
        accepted {formatCount(accepted)} · degraded: {degraded ? 'yes' : 'no'}
      </p>
      {problems.length > 0 && (
        <ul>
          {problems.map((p) => (
            <li key={p.key}>
              {formatCount(p.count)} {p.label}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
