import { Link } from '../components/Layout';
import './overview.css';
import { firstSafeTargetName } from './overviewModel';
import {
  CMD_INIT,
  CMD_REPLAY,
  CMD_TARGETS,
  GUIDE_HEADING,
  GUIDE_INTRO,
  GUIDE_NOT_A_RESULT,
  GUIDE_PROCESS_NOTE,
  GUIDE_RUNS_LINK,
  GUIDE_STEP_BROWSER,
  GUIDE_STEP_INIT,
  GUIDE_STEP_LIST,
  GUIDE_STEP_REPLAY,
  GUIDE_STEP_RUN,
  GUIDE_TARGETS_NONE,
  GUIDE_TARGETS_UNKNOWN,
  TARGET_PLACEHOLDER,
  cmdTest,
  guideTargets,
} from './overviewWording';

const NAMES_SHOWN = 5;

interface Props {
  /** The targets the server lists, or undefined while they are not known (still loading, or the load failed). */
  readonly targets: readonly { name: string }[] | undefined;
  /** The targets endpoint failed: the guide then says so instead of claiming there are none. */
  readonly targetsFailed: boolean;
}

function targetsLine(targets: Props['targets'], failed: boolean): string | undefined {
  if (targets === undefined) return failed ? GUIDE_TARGETS_UNKNOWN : undefined;
  if (targets.length === 0) return GUIDE_TARGETS_NONE;
  return guideTargets(
    targets.slice(0, NAMES_SHOWN).map((t) => t.name),
    targets.length,
  );
}

function Step({ text, command }: { text: string; command?: string }) {
  return (
    <li>
      <span>{text}</span>
      {command !== undefined && <pre className="guide-command"><code>{command}</code></pre>}
    </li>
  );
}

/**
 * What a newcomer does next, shown when the server has no stored runs. The commands are the ones docs/quickstart.md and the
 * README give, and the only target name used is one the server itself lists. It makes no claim about what a run will find.
 */
export function FirstRunGuide({ targets, targetsFailed }: Props) {
  const name = targets === undefined ? undefined : firstSafeTargetName(targets);
  const line = targetsLine(targets, targetsFailed);
  return (
    <section className="guide card" aria-labelledby="overview-guide">
      <h2 id="overview-guide">{GUIDE_HEADING}</h2>
      <p>{GUIDE_INTRO}</p>
      {line !== undefined && <p className="muted">{line}</p>}
      <ol className="guide-steps">
        <Step text={GUIDE_STEP_LIST} command={CMD_TARGETS} />
        <Step text={GUIDE_STEP_INIT} command={CMD_INIT} />
        <Step text={GUIDE_STEP_RUN} command={cmdTest(name ?? TARGET_PLACEHOLDER)} />
        <Step text={GUIDE_STEP_REPLAY} command={CMD_REPLAY} />
        <Step text={GUIDE_STEP_BROWSER} />
      </ol>
      <p>
        <Link to="/runs">{GUIDE_RUNS_LINK}</Link>
      </p>
      <p className="muted">{GUIDE_NOT_A_RESULT}</p>
      <p className="muted">{GUIDE_PROCESS_NOTE}</p>
    </section>
  );
}
