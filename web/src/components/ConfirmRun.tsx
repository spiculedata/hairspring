import { useEffect, useRef, useState } from 'react';
import { api, ApiError } from '../api';
import {
  CONFIRM_STEP_TITLE,
  FINISHED_TEXT,
  NOT_NOW,
  RUN_CONFIRMATION,
  START_CONFIRMATION,
  STARTING_CONFIRMATION,
  confirmStepText,
  busyText,
  runningText,
} from '../confirmationWording';
import { confirmAccess, type ConfirmLimits } from '../me';
import { useMe } from '../meStore';
import { usePolling } from '../usePolling';
import './confirmation.css';
import { ErrorLine } from './ErrorLine';

const POLL_MS = 3000;

/**
 * "Run confirmation", for a caller the server says may start one (`can.confirm` in `/api/me`); for anyone else it shows
 * nothing, and the page keeps the command to run. Starting asks first, and says what it costs. The server answers at
 * once and does the work in the background; this checks its status until it is no longer running, then asks the page to
 * load the stored checks. It never says a failure is confirmed: the ladder says that, from what the server stored.
 */
export function ConfirmRun({ failureId, onFinished }: { failureId: string; onFinished: () => void }) {
  const limits = confirmAccess(useMe());
  return limits === undefined ? null : <ConfirmRunPanel failureId={failureId} limits={limits} onFinished={onFinished} />;
}

type Phase = 'idle' | 'asking' | 'starting';

function ConfirmRunPanel({ failureId, limits, onFinished }: { failureId: string; limits: ConfirmLimits; onFinished: () => void }) {
  const [phase, setPhase] = useState<Phase>('idle');
  const [refusal, setRefusal] = useState<string>();
  const [justFinished, setJustFinished] = useState(false);
  const inFlight = useRef(false);
  const runButton = useRef<HTMLButtonElement>(null);
  const step = useRef<HTMLDivElement>(null);
  const statusRegion = useRef<HTMLDivElement>(null);
  // The control is a new element after the question closes, so focus goes back to it once it is there.
  const returnFocus = useRef(false);
  const status = usePolling(() => api.confirmStatus(failureId), POLL_MS, (s) => s.state !== 'running');
  const running = status.data?.state === 'running';
  const wasRunning = useRef(false);

  useEffect(() => {
    if (running) {
      wasRunning.current = true;
      return;
    }
    if (wasRunning.current && status.data !== undefined) {
      wasRunning.current = false;
      setJustFinished(status.data.state === 'idle');
      onFinished();
    }
  }, [running, status.data, onFinished]);

  useEffect(() => {
    if (phase === 'asking') step.current?.focus();
    if (phase === 'idle' && returnFocus.current) {
      returnFocus.current = false;
      runButton.current?.focus();
    }
  }, [phase]);

  async function start() {
    if (inFlight.current) return;
    inFlight.current = true;
    setPhase('starting');
    setRefusal(undefined);
    setJustFinished(false);
    try {
      await api.startConfirmation(failureId);
      wasRunning.current = true;
      status.reload();
      setPhase('idle');
      // The question is gone and the control is disabled while it runs, so focus goes to the place that says so.
      statusRegion.current?.focus();
    } catch (e) {
      setRefusal(refusalText(e));
      setPhase('idle');
    } finally {
      inFlight.current = false;
    }
  }

  function dismiss() {
    returnFocus.current = true;
    setPhase('idle');
  }

  return (
    <div className="confirm-run">
      {phase === 'asking' || phase === 'starting' ? (
        <div className="confirm-step" role="group" aria-labelledby="confirm-step-title" tabIndex={-1} ref={step}>
          <p id="confirm-step-title" className="confirm-step-title">
            {CONFIRM_STEP_TITLE}
          </p>
          <p>{confirmStepText(limits.wall_secs)}</p>
          <div className="confirm-step-actions">
            <button type="button" onClick={start} disabled={phase === 'starting'}>
              {phase === 'starting' ? STARTING_CONFIRMATION : START_CONFIRMATION}
            </button>
            <button type="button" onClick={dismiss} disabled={phase === 'starting'}>
              {NOT_NOW}
            </button>
          </div>
        </div>
      ) : (
        <button type="button" ref={runButton} onClick={() => setPhase('asking')} disabled={running}>
          {RUN_CONFIRMATION}
        </button>
      )}
      {refusal !== undefined && <ErrorLine message={refusal} />}
      <div role="status" tabIndex={-1} ref={statusRegion}>
        {running && <p>{runningText(limits.wall_secs)}</p>}
        {!running && status.data?.state === 'failed' && status.data.reason !== undefined && <p className="error">{status.data.reason}</p>}
        {!running && justFinished && <p>{FINISHED_TEXT}</p>}
      </div>
    </div>
  );
}

/** The server's own words for a refusal; for a 429, that it is full and when to try again. */
function refusalText(e: unknown): string {
  if (e instanceof ApiError && e.status === 429) return busyText(e.retryAfterMs);
  return e instanceof Error ? e.message : String(e);
}
