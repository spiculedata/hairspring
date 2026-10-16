import { caveatLabel, CHIP_LABEL_NOT_DECLARED, CHIP_UNVERIFIED } from '../wording';

type Tone = 'muted' | 'warn' | 'info';

interface Chip {
  readonly text: string;
  readonly tone: Tone;
}

interface Props {
  /** The label chips the server generated from the run's declared label. */
  chips: readonly string[];
  /** Caveat codes the server attached, such as `single_run`. */
  caveats: readonly string[];
  /** The header says some worker data is not from a trusted worker. */
  unverified: boolean;
}

function toneOf(text: string): Tone {
  if (text === CHIP_LABEL_NOT_DECLARED) return 'muted';
  if (text === CHIP_UNVERIFIED) return 'warn';
  return 'info';
}

/**
 * Every chip a run header shows. A run without a declared label always shows the grey "label not declared" chip, and a
 * run with untrusted worker data always shows "unverified worker data", whether or not the server's list had it.
 * Each chip is words; the tone class adds emphasis and is never the only carrier of meaning.
 */
export function chipList({ chips, caveats, unverified }: Props): Chip[] {
  const texts = [...chips, ...caveats.map(caveatLabel)];
  const hasLabel = texts.some((t) => t === CHIP_LABEL_NOT_DECLARED || t.startsWith('tier:'));
  const withLabel = hasLabel ? texts : [CHIP_LABEL_NOT_DECLARED, ...texts];
  const withTrust = unverified ? [...withLabel, CHIP_UNVERIFIED] : withLabel;
  const unique = [...new Set(withTrust)];
  return unique.map((text) => ({ text, tone: caveats.map(caveatLabel).includes(text) ? 'warn' : toneOf(text) }));
}

export function TierChips(props: Props) {
  return (
    <ul className="chips" aria-label="Run label and caveats">
      {chipList(props).map((c) => (
        <li key={c.text} className={`chip chip-${c.tone}`}>
          {c.text}
        </li>
      ))}
    </ul>
  );
}
