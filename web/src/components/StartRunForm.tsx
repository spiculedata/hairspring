import { useRef, useState, type FormEvent } from 'react';
import { buildStartRun, enforcedBudget, startErrorText, type RunMode, type StartFormErrors, type StartRunBody } from '../startRun';
import {
  BUDGET_HEADING,
  RUN_TYPE_LEGEND,
  SEARCH_EXPLAINED,
  SEARCH_LABEL,
  START_RUN,
  START_SWEEP,
  STARTING,
  SWEEP_EXPLAINED,
  SWEEP_LABEL,
  TRIALS_LABEL,
  TRIALS_PLACEHOLDER,
} from '../startRunWording';
import { runPath } from '../router';
import type { Budget, Run, Target } from '../types';
import { plural } from '../wording';

interface Props {
  readonly targets: readonly Target[];
  readonly start: (body: StartRunBody) => Promise<{ run: Pick<Run, 'id'> }>;
  /** Called with the path of the new run's page. */
  readonly onStarted: (path: string) => void;
}

function BudgetFacts({ budget }: { budget: Budget }) {
  return (
    <div className="start-budget">
      <h2>{BUDGET_HEADING}</h2>
      <dl className="facts">
        <dt>Trial budget</dt>
        <dd>{plural(budget.max_trials, 'trial', 'trials')}</dd>
        <dt>Step budget</dt>
        <dd>{plural(budget.max_steps, 'step', 'steps')}</dd>
        <dt>Wall-clock budget</dt>
        <dd>{plural(budget.wall_secs, 'second', 'seconds')}</dd>
      </dl>
    </div>
  );
}

function FieldError({ id, message }: { id: string; message: string | undefined }) {
  return message === undefined ? null : (
    <span id={id} className="error">
      {message}
    </span>
  );
}

/** The start-run form: target, search or sweep, an optional seed and, for a sweep, the number of trials. */
export function StartRunForm({ targets, start, onStarted }: Props) {
  const [targetName, setTargetName] = useState('');
  const [mode, setMode] = useState<RunMode>('search');
  const [seedText, setSeedText] = useState('');
  const [trialsText, setTrialsText] = useState('');
  const [errors, setErrors] = useState<StartFormErrors>({});
  const [message, setMessage] = useState<string>();
  const [pending, setPending] = useState(false);
  // State updates are not seen by a second submit in the same tick, so the guard against a double start is a ref.
  const inFlight = useRef(false);
  const target = targets.find((t) => t.name === targetName) ?? targets[0];

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (target === undefined || inFlight.current) return;
    const built = buildStartRun({ target, mode, seedText, trialsText });
    setMessage(undefined);
    if (!built.ok) return setErrors(built.errors);
    setErrors({});
    inFlight.current = true;
    setPending(true);
    try {
      const { run } = await start(built.body);
      onStarted(runPath(run.id));
    } catch (err) {
      setMessage(startErrorText(err));
    } finally {
      inFlight.current = false;
      setPending(false);
    }
  }

  const trials = mode === 'sweep' && /^\d+$/.test(trialsText.trim()) ? Number(trialsText.trim()) : undefined;
  const shown = target === undefined ? undefined : enforcedBudget(target.budget, mode, trials);
  const isSweep = mode === 'sweep';
  return (
    <form className="card start-run" onSubmit={submit} noValidate>
      <label>
        Target
        <select value={target?.name ?? ''} onChange={(e) => setTargetName(e.target.value)}>
          {targets.map((t) => (
            <option key={t.name} value={t.name}>
              {t.name} ({t.kind})
            </option>
          ))}
        </select>
      </label>
      <fieldset>
        <legend>{RUN_TYPE_LEGEND}</legend>
        <label className="choice">
          <input type="radio" name="mode" checked={!isSweep} onChange={() => setMode('search')} />
          {SEARCH_LABEL}
        </label>
        <label className="choice">
          <input type="radio" name="mode" checked={isSweep} onChange={() => setMode('sweep')} />
          {SWEEP_LABEL}
        </label>
      </fieldset>
      <div className="start-explain muted">
        {isSweep ? SWEEP_EXPLAINED.map((s) => <p key={s}>{s}</p>) : <p>{SEARCH_EXPLAINED}</p>}
      </div>
      <label>
        Seed (optional)
        <input
          name="seed"
          inputMode="numeric"
          placeholder="random"
          value={seedText}
          aria-invalid={errors.seed !== undefined}
          aria-describedby={errors.seed === undefined ? undefined : 'seed-error'}
          onChange={(e) => setSeedText(e.target.value)}
        />
        <FieldError id="seed-error" message={errors.seed} />
      </label>
      {isSweep && (
        <label>
          {TRIALS_LABEL}
          <input
            name="trials"
            inputMode="numeric"
            placeholder={TRIALS_PLACEHOLDER}
            value={trialsText}
            aria-invalid={errors.trials !== undefined}
            aria-describedby={errors.trials === undefined ? undefined : 'trials-error'}
            onChange={(e) => setTrialsText(e.target.value)}
          />
          <FieldError id="trials-error" message={errors.trials} />
        </label>
      )}
      {shown !== undefined && <BudgetFacts budget={shown} />}
      <div className="row">
        <button type="submit" disabled={target === undefined || pending}>
          {pending ? STARTING : isSweep ? START_SWEEP : START_RUN}
        </button>
      </div>
      {message !== undefined && (
        <p className="error" role="alert">
          {message}
        </p>
      )}
    </form>
  );
}
