import { GuestText } from '../components/GuestText';
import { formatCount, NO_FAILING_PROPERTY, PER_THREAD_NOTE, witnessText, WITNESS_VALUES_NOTE } from '../wording';
import { ordinalOf, witnessThread, type FailingProperty } from './model';
import type { KeptProc, Witness } from './types';

/** Failing properties listed before the rest are summarised as a count. */
const MAX_LISTED = 20;
const ID_CHARS = 160;
const MSG_CHARS = 300;
const LOC_CHARS = 120;
const NAME_CHARS = 80;

const valuesOf = (w: Witness): string => w.vals.map((v) => (Number.isFinite(v) ? String(v) : '?')).join(' ');

function WitnessLine({ w, procs }: { w: Witness; procs: readonly KeptProc[] }) {
  const thread = witnessThread(procs, w);
  const key = thread === undefined ? 'thread not reported' : thread.key_stable ? 'stable key' : 'unstable key';
  return (
    <p className="witness-line">
      proc <GuestText text={w.proc} max={NAME_CHARS} expandable={false} /> · thread <GuestText text={w.thread} max={NAME_CHARS} expandable={false} /> ({key}) ·{' '}
      {witnessText(w.ord, ordinalOf(w.ord), thread?.evals)} · values <code>{valuesOf(w)}</code>
    </p>
  );
}

function Failing({ f, procs }: { f: FailingProperty; procs: readonly KeptProc[] }) {
  const { property } = f;
  const what = f.reason === 'always_false' ? 'was false' : 'was reached';
  return (
    <li>
      <p>
        <strong>{property.kind.replace(/_/g, ' ')}</strong> <GuestText text={property.id} max={ID_CHARS} />{' '}
        <span className="muted">
          {what} {formatCount(f.count)} {f.count === 1 ? 'time' : 'times'}
        </span>
      </p>
      {property.msg !== '' && (
        <p className="prop-sub">
          <GuestText text={property.msg} max={MSG_CHARS} />
          {property.loc !== null && (
            <>
              {' '}
              <GuestText text={property.loc.file} max={LOC_CHARS} expandable={false} />
              {`:${property.loc.line}`}
              {property.loc.func !== null && (
                <>
                  {' '}
                  <GuestText text={property.loc.func} max={LOC_CHARS} expandable={false} />
                </>
              )}
            </>
          )}
        </p>
      )}
      {f.witness === null ? <p className="muted">no witness was recorded for this property</p> : <WitnessLine w={f.witness} procs={procs} />}
    </li>
  );
}

/** Each failing property with its witness: the thread, that thread's own evaluation number, and the recorded values. */
export function WitnessSection({ failing, procs }: { failing: readonly FailingProperty[]; procs: readonly KeptProc[] }) {
  return (
    <section aria-labelledby="witness-title">
      <h2 id="witness-title">Witness</h2>
      <p className="muted">
        {PER_THREAD_NOTE} The four values are {WITNESS_VALUES_NOTE}.
      </p>
      {failing.length === 0 ? (
        <p>{NO_FAILING_PROPERTY}</p>
      ) : (
        <ul className="failing">
          {failing.slice(0, MAX_LISTED).map((f) => (
            <Failing key={f.property.id} f={f} procs={procs} />
          ))}
        </ul>
      )}
      {failing.length > MAX_LISTED && <p className="muted">{formatCount(failing.length - MAX_LISTED)} more failing properties not listed.</p>}
    </section>
  );
}
