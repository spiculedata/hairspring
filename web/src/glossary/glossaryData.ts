/**
 * The one list of definitions the Glossary page and every `Term` read. The wording is the wiki Glossary's
 * (https://github.com/spiculedata/hairspring/wiki/Glossary), shortened where the page is not the place for detail, and
 * agrees with docs/determinism.md and docs/architecture.md. `wiki` says where the same meaning is written in the wiki,
 * so a reader of a change can compare the two. Nothing here says a failure is fixed or counts a rate.
 */

interface Entry {
  readonly term: string;
  readonly definition: string;
  readonly wiki: string;
}

export const GLOSSARY = {
  bundle: {
    term: 'Bundle',
    definition: 'A small text file that records what is needed to replay one failing trial: seed, build, target, backend, parameters and path.',
    wiki: 'Glossary: Bundle',
  },
  confirmation: {
    term: 'Confirmation',
    definition:
      'Recorded checks that a failure repeats: same-seed reruns, bundle replay, and optionally a fixed-build control and Hermit\'s strict verify. Run with hairspring confirm.',
    wiki: 'Glossary: Confirmation',
  },
  control: {
    term: 'Control',
    definition: 'A second target running the fixed build, used to check that the fix is clean at the failing seed.',
    wiki: 'Glossary: Control',
  },
  deterministic: {
    term: 'Deterministic',
    definition: 'Same inputs, including the seed, give the same execution.',
    wiki: 'Glossary: Deterministic',
  },
  finding: {
    term: 'Finding',
    definition:
      'A recorded failure and its evidence. Its state (candidate, rerun_repeated, confirmed, verified, not_reproducible, control_failed) is derived from stored checks.',
    wiki: 'Glossary: Finding',
  },
  hit: {
    term: 'Hit',
    definition: 'A trial in which a property failed. Reports say hits, not bugs.',
    wiki: 'Glossary: Hit',
  },
  inconclusive: {
    term: 'Inconclusive',
    definition: 'A trial that could not be judged, such as a timeout. Never a pass.',
    wiki: 'Glossary: Inconclusive',
  },
  infra: {
    term: 'Infra',
    definition: 'A trial that failed because of the host or Hermit, not the program. Listed apart; never a pass.',
    wiki: 'Glossary: Infra',
  },
  property: {
    term: 'Property',
    definition: 'A rule the system must obey. Kinds: always, always_or_unreachable, sometimes, reachable, unreachable.',
    wiki: 'Glossary: Property',
  },
  'rated-trial': {
    term: 'Rated trial',
    definition: 'A trial that counts toward a rate: a pass or a violation, with usable events.',
    wiki: 'Glossary: Rated trial',
  },
  replay: {
    term: 'Replay',
    definition: 'Running a bundle again to see whether the failure repeats.',
    wiki: 'Glossary: Replay',
  },
  run: {
    term: 'Run',
    definition: 'One job that tests a target with many trials. A search or a sweep.',
    wiki: 'Glossary: Run',
  },
  schedule: {
    term: 'Schedule',
    definition: 'The order in which things happened in a trial. Compared by a schedule fingerprint on the process backend.',
    wiki: 'Glossary: Schedule',
  },
  search: {
    term: 'Search',
    definition: 'The default run mode. Stops at the first confirmed failure. Gives counts, not a rate.',
    wiki: 'Glossary: Search',
  },
  seed: {
    term: 'Seed',
    definition: 'A number that drives every choice in a trial. Same seed, same choices.',
    wiki: 'Glossary: Seed',
  },
  'strict-verify': {
    term: 'Strict verify',
    definition:
      'A confirmation check made with Hermit\'s verify mode, which runs the program twice and compares the two runs. Run with hairspring confirm --strict-verify.',
    wiki: 'Glossary: Confirmation, and docs/determinism.md',
  },
  sweep: {
    term: 'Sweep',
    definition: 'A run that runs its whole trial budget and stores every trial, so it can state a rate.',
    wiki: 'Glossary: Sweep',
  },
  target: {
    term: 'Target',
    definition: 'A named thing to test, with its kind and budget.',
    wiki: 'Glossary: Target',
  },
  trial: {
    term: 'Trial',
    definition: 'One attempt to make a target fail: one seed (process, wasm) or one exploration round (sim).',
    wiki: 'Glossary: Trial',
  },
  unusable: {
    term: 'Unusable',
    definition: 'A trial whose reported events were lost, cut or contradictory. Never a pass.',
    wiki: 'Glossary: Unusable',
  },
  verdict: {
    term: 'Verdict',
    definition: 'How a trial ended: pass, violation, inconclusive, unusable or infra.',
    wiki: 'Glossary: Verdict',
  },
  witness: {
    term: 'Witness',
    definition: 'Up to four integers the program recorded with an evaluation of a property, kept from the first true and the first false evaluation.',
    wiki: 'Guest-SDKs-by-Language: witness values',
  },
} as const satisfies Readonly<Record<string, Entry>>;

export type GlossaryKey = keyof typeof GLOSSARY;

export interface GlossaryEntry extends Entry {
  readonly key: GlossaryKey;
}

export const GLOSSARY_KEYS = Object.keys(GLOSSARY) as readonly GlossaryKey[];

/** Whether `key` names an entry. Own keys only, so a word like `constructor` is not one. */
export const isGlossaryKey = (key: string): key is GlossaryKey => Object.hasOwn(GLOSSARY, key);

/** Every entry, in the order of the terms as a reader sorts them. */
export function glossaryEntries(): readonly GlossaryEntry[] {
  return GLOSSARY_KEYS.map((key) => ({ key, ...GLOSSARY[key] })).sort((a, b) => a.term.localeCompare(b.term, 'en-GB'));
}
