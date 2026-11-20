#!/usr/bin/env node
// Change classification: which risk tier does a pull request fall into?
//
// The rules live in .github/risk-tiers.yml so a human can read and edit them
// without touching this file, and so the same rules can be quoted in a review.
// This script is the thing that applies them.
//
// Two properties matter more than the classification itself:
//
//   1. It is deterministic and offline. Given the same file list and the same
//      config it always produces the same answer, so it can be unit tested, and
//      so it never needs a network call or a token to decide what a change is.
//   2. It is advisory. Nothing here blocks a merge. It labels the pull request
//      and says what the tier's review requirement is; the judgement of whether
//      that was met stays with the reviewer. A classifier that could block a
//      merge would be a gate, and a gate that cannot be argued with is worse
//      than no gate.
//
// Input is paths only. The script never reads a PR title, body, branch name or
// comment, so there is nothing in it for a contributor to prompt-inject.
//
// Why a YAML parser lives here rather than a dependency
// ------------------------------------------------------
// The tier-classifier workflow runs this with only a checkout and Node: no
// `npm install`, so no node_modules. `parseYamlSubset` below handles the block
// subset the config is written in (nested mappings, sequences, scalars, folded
// block scalars, comments) and throws on anything else (flow style, anchors,
// tabs) rather than guessing. A config that cannot be parsed is a config that
// cannot be silently misread.
//
// Usage:
//   classify-change.mjs --config .github/risk-tiers.yml --files changed.txt
//   classify-change.mjs --config .github/risk-tiers.yml --files changed.txt --json
//   classify-change.mjs --check-config --config .github/risk-tiers.yml

import { readFileSync } from "node:fs";
import path from "node:path";

// --- a deliberately small YAML subset --------------------------------------

const UNSUPPORTED = [
  ["&", "anchors"],
  ["*", "aliases"],
  ["{", "flow mappings"],
  ["[", "flow sequences"],
];

function stripComment(line) {
  let single = false;
  let double = false;
  for (let i = 0; i < line.length; i += 1) {
    const c = line[i];
    if (c === "'" && !double) single = !single;
    else if (c === '"' && !single) double = !double;
    else if (c === "#" && !single && !double && (i === 0 || /\s/.test(line[i - 1]))) {
      return line.slice(0, i);
    }
  }
  return line;
}

function scalar(raw, where) {
  const text = raw.trim();
  if (text === "") return "";
  const first = text[0];
  if (first === '"' || first === "'") {
    if (text.length < 2 || text[text.length - 1] !== first) {
      throw new Error(`${where}: unterminated quoted string ${text}`);
    }
    return text.slice(1, -1);
  }
  for (const [token, name] of UNSUPPORTED) {
    if (first === token) throw new Error(`${where}: ${name} are not supported (${text})`);
  }
  if (first === "?" || first === "&") throw new Error(`${where}: unsupported line ${text}`);
  if (text === "true") return true;
  if (text === "false") return false;
  if (text === "null" || text === "~") return null;
  if (/^-?\d+$/.test(text)) return Number(text);
  // A plain scalar cannot contain ": ". Real YAML rejects it ("mapping values
  // are not allowed here"), and quietly accepting it here would mean the config
  // parses in CI and fails in the workflow, which is the worst of both.
  if (/:\s/.test(text) || text.endsWith(":")) {
    throw new Error(`${where}: unquoted value contains ": " — quote the whole value (${text})`);
  }
  return text;
}

/** The index of the ":" that ends a key, or -1. Quotes are respected. */
function keyColon(text) {
  let single = false;
  let double = false;
  for (let i = 0; i < text.length; i += 1) {
    const c = text[i];
    if (c === "'" && !double) single = !single;
    else if (c === '"' && !single) double = !double;
    else if (c === ":" && !single && !double) {
      if (i === text.length - 1 || /\s/.test(text[i + 1])) return i;
    }
  }
  return -1;
}

function tokenize(text) {
  const tokens = [];
  const lines = text.split(/\r?\n/);
  for (let n = 0; n < lines.length; n += 1) {
    const raw = lines[n];
    if (/^\s*\t/.test(raw) || raw.includes("\t")) {
      throw new Error(`line ${n + 1}: tabs are not indentation in this config`);
    }
    const stripped = stripComment(raw);
    if (stripped.trim() === "") continue;
    const indent = stripped.length - stripped.trimStart().length;
    if (indent % 2 !== 0) {
      throw new Error(`line ${n + 1}: indent must be a multiple of two spaces`);
    }
    const body = stripped.trim();

    // A folded block scalar (`key: >-`) owns every following deeper line. It is
    // consumed here, from the raw text, because a `#` inside one of those lines
    // is content rather than a comment — stripping comments first would quietly
    // shorten a sentence. Only `>` (folded, chomped) and `>`/`|` with an
    // optional chomping or explicit indentation indicator are accepted.
    const block = body.match(/^(.*?):\s*([>|])[-+\d]*\s*$/);
    if (block) {
      const parts = [];
      let m = n + 1;
      while (m < lines.length) {
        const next = lines[m];
        if (next.trim() === "") {
          parts.push("");
          m += 1;
          continue;
        }
        const nextIndent = next.length - next.trimStart().length;
        if (nextIndent <= indent) break;
        parts.push(next.trim());
        m += 1;
      }
      while (parts.length > 0 && parts[parts.length - 1] === "") parts.pop();
      tokens.push({
        indent,
        text: `${block[1].trim()}:`,
        line: n + 1,
        block: foldBlock(parts),
      });
      n = m - 1;
      continue;
    }

    tokens.push({ indent, text: body, line: n + 1 });
  }
  return tokens;
}

/** Fold a block scalar the way `>` does: a blank line is a paragraph break. */
function foldBlock(parts) {
  const out = [];
  for (const part of parts) {
    if (part === "") out.push("\n");
    else if (out.length > 0 && !out[out.length - 1].endsWith("\n")) out[out.length - 1] += ` ${part}`;
    else out.push(part);
  }
  return out
    .map((chunk) => (chunk === "\n" ? chunk : chunk.replace(/\s+/g, " ").trim()))
    .join("")
    .trim();
}

function parseBlock(tokens, start, indent) {
  const isSequence = () => tokens[start] && tokens[start].text.startsWith("- ") || tokens[start]?.text === "-";
  if (!isSequence()) return parseMapping(tokens, start, indent);
  return parseSequence(tokens, start, indent);
}

function parseMapping(tokens, start, indent) {
  const map = {};
  let i = start;
  while (i < tokens.length && tokens[i].indent === indent) {
    const token = tokens[i];
    if (token.text.startsWith("- ") || token.text === "-") {
      throw new Error(`line ${token.line}: a sequence cannot sit at a mapping's indent`);
    }
    const colon = keyColon(token.text);
    if (colon < 0) throw new Error(`line ${token.line}: expected "key: value", got ${token.text}`);
    const key = token.text.slice(0, colon).trim();
    if (key === "") throw new Error(`line ${token.line}: empty key`);
    if (Object.hasOwn(map, key)) throw new Error(`line ${token.line}: duplicate key ${key}`);
    const rest = token.text.slice(colon + 1).trim();
    if (token.block !== undefined) {
      map[key] = token.block;
      i += 1;
    } else if (rest !== "") {
      map[key] = scalar(rest, `line ${token.line}`);
      i += 1;
    } else if (i + 1 < tokens.length && tokens[i + 1].indent > indent) {
      const [value, next] = parseBlock(tokens, i + 1, tokens[i + 1].indent);
      map[key] = value;
      i = next;
    } else {
      map[key] = null;
      i += 1;
    }
  }
  return [map, i];
}

function parseSequence(tokens, start, indent) {
  const items = [];
  let i = start;
  while (
    i < tokens.length &&
    tokens[i].indent === indent &&
    (tokens[i].text === "-" || tokens[i].text.startsWith("- "))
  ) {
    const token = tokens[i];
    const rest = token.text === "-" ? "" : token.text.slice(2).trim();
    if (rest === "") {
      if (i + 1 >= tokens.length || tokens[i + 1].indent <= indent) {
        items.push(null);
        i += 1;
      } else {
        const [value, next] = parseBlock(tokens, i + 1, tokens[i + 1].indent);
        items.push(value);
        i = next;
      }
    } else if (keyColon(rest) >= 0) {
      // "- key: value" is the first line of a mapping whose indent is the
      // column the key starts at. Re-indent the token to that column and let
      // the ordinary mapping parser read it along with its continuation lines.
      const inner = indent + 2;
      tokens[i] = { indent: inner, text: rest, line: token.line, block: token.block };
      const [value, next] = parseMapping(tokens, i, inner);
      items.push(value);
      i = next;
    } else {
      items.push(scalar(rest, `line ${token.line}`));
      i += 1;
    }
  }
  return [items, i];
}

/** Parse the block-mapping/block-sequence subset this repository's config uses. */
export function parseYamlSubset(text) {
  const tokens = tokenize(text);
  if (tokens.length === 0) return {};
  const [value, next] = parseBlock(tokens, 0, tokens[0].indent);
  if (next !== tokens.length) {
    throw new Error(`line ${tokens[next].line}: unexpected indentation`);
  }
  return value;
}

// --- glob matching ----------------------------------------------------------

/** Globs behave the way a reader expects: `*` stops at a slash, `**` does not. */
export function globToRegExp(glob) {
  let out = "^";
  for (let i = 0; i < glob.length; i += 1) {
    const c = glob[i];
    if (c === "*") {
      if (glob[i + 1] === "*") {
        out += ".*";
        i += 1;
        if (glob[i + 1] === "/") i += 1; // "**/" also matches zero directories
      } else {
        out += "[^/]*";
      }
    } else if (c === "?") out += "[^/]";
    else if ("\\^$.|+()[]{}".includes(c)) out += `\\${c}`;
    else out += c;
  }
  return new RegExp(`${out}$`);
}

// --- the configuration ------------------------------------------------------

function requireField(value, field, where) {
  if (value === undefined || value === null || value === "") {
    throw new Error(`${where}: ${field} is required`);
  }
  return value;
}

/**
 * Read and validate the tier config. Validation is strict on purpose: a typo in
 * a path pattern would silently lower a change's tier, which is the one
 * failure this file exists to prevent.
 */
export function loadConfig(text) {
  const raw = parseYamlSubset(text);
  const where = ".github/risk-tiers.yml";
  requireField(raw, "version", where);
  const tiers = raw.tiers;
  if (!Array.isArray(tiers) || tiers.length === 0) {
    throw new Error(`${where}: tiers must be a non-empty sequence`);
  }
  const seenIds = new Set();
  const seenLabels = new Set();
  const parsed = tiers.map((tier, index) => {
    const at = `${where} tier #${index + 1}`;
    const id = requireField(tier.id, "id", at);
    const label = requireField(tier.label, "label", at);
    if (seenIds.has(id)) throw new Error(`${at}: duplicate tier id ${id}`);
    if (seenLabels.has(label)) throw new Error(`${at}: duplicate tier label ${label}`);
    seenIds.add(id);
    seenLabels.add(label);
    const rank = tier.rank;
    if (typeof rank !== "number") throw new Error(`${at}: rank must be a number`);
    const paths = requireField(tier.paths, "paths", at);
    if (!Array.isArray(paths) || paths.length === 0) {
      throw new Error(`${at}: paths must be a non-empty sequence`);
    }
    return {
      id,
      label,
      rank,
      summary: requireField(tier.summary, "summary", at),
      review: requireField(tier.review, "review", at),
      paths: paths.map((entry, n) => {
        const at2 = `${at} path #${n + 1}`;
        const match = requireField(entry.match, "match", at2);
        // `pending: true` marks a pattern for a file that does not exist yet (or only
        // exists on another branch); the dead-pattern test skips it.
        return { match, why: requireField(entry.why, "why", at2), pending: entry.pending === true, re: globToRegExp(match) };
      }),
    };
  });
  const defaultId = requireField(raw.default_tier, "default_tier", where);
  const fallback = parsed.find((t) => t.id === defaultId);
  if (!fallback) throw new Error(`${where}: default_tier ${defaultId} is not a tier`);
  return { version: raw.version, tiers: parsed, defaultTier: fallback };
}

/**
 * Classify a list of changed paths. The tier is the highest rank any file
 * matched, so a pull request that touches one migration file is critical even if
 * the other forty files are documentation — which is the intended behaviour, and
 * why the answer is a maximum rather than the first match.
 *
 * Overlapping patterns are normal: lib/db/schema.ts is also under
 * `lib/**`. Each file reports only its own highest-ranked tier, so the
 * reason shown to a reviewer is the one that decided the answer rather than every
 * pattern that happened to be true.
 */
export function classify(config, files) {
  // The workflow feeds this exactly what `gh api .../files` prints, which is a
  // forward-slashed path with surrounding whitespace on some platforms. Normalise
  // first so a path cannot be silently dropped out of the classification by a
  // separator or a stray space.
  const clean = [...new Set(files.map((f) => f.trim().replace(/\\/g, "/").replace(/^\.\//, "")).filter(Boolean))];
  const matches = [];
  let tier = null;
  for (const file of clean) {
    let best = null;
    for (const candidate of config.tiers) {
      const hit = candidate.paths.find((p) => p.re.test(file));
      if (!hit) continue;
      if (!best || candidate.rank > best.rank) best = { tier: candidate, why: hit.why };
    }
    if (!best) continue;
    matches.push({ path: file, tier: best.tier.id, why: best.why });
    if (!tier || best.tier.rank > tier.rank) tier = best.tier;
  }
  // A file nobody thought hard about still gets a stated expectation, which is
  // what makes default_tier worth having: it is a real tier, not a shrug.
  return { tier: tier ?? config.defaultTier, matchedDefault: tier === null, matches, files: clean };
}

/** The sticky comment body. The marker is what the workflow upserts on. */
export function renderComment(result) {
  const { tier, matches, files } = result;
  const lines = [
    "<!-- change-classification -->",
    `**Change classification: \`${tier.id}\`** (label \`${tier.label}\`)`,
    "",
    tier.summary,
    "",
    `**Review requirement.** ${tier.review}`,
    "",
    `<sub>${files.length} file(s) changed. Classification is advisory: it does not block a merge, and it is only as good as .github/risk-tiers.yml. Edit that file to change what a tier means.</sub>`,
  ];
  if (matches.length > 0) {
    lines.push("", "<details><summary>What set the tier</summary>", "", "| File | Tier | Because |", "| --- | --- | --- |");
    for (const m of matches) lines.push(`| \`${m.path}\` | ${m.tier} | ${m.why} |`);
    lines.push("", "</details>");
  }
  return lines.join("\n");
}

function parseArgs(argv) {
  const args = { files: [] };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--config") args.config = argv[++i];
    else if (arg === "--files") args.files.push(argv[++i]);
    else if (arg === "--json") args.json = true;
    else if (arg === "--markdown") args.markdown = true;
    else if (arg === "--check-config") args.checkConfig = true;
    else throw new Error(`unknown argument ${arg}`);
  }
  return args;
}

export function main(argv, readFile = readFileSync) {
  const args = parseArgs(argv);
  const configPath = args.config ?? ".github/risk-tiers.yml";
  const config = loadConfig(readFile(configPath, "utf8"));
  if (args.checkConfig) {
    process.stdout.write(`${configPath}: ${config.tiers.length} tiers, default ${config.defaultTier.id}\n`);
    return 0;
  }
  if (args.files.length === 0) throw new Error("--files is required (a file containing changed paths)");
  const files = args.files.flatMap((f) => readFile(f, "utf8").split("\n"));
  const result = classify(config, files);
  if (args.markdown) {
    process.stdout.write(`${renderComment(result)}\n`);
  } else {
    process.stdout.write(
      `${JSON.stringify(
        {
          tier: result.tier.id,
          label: result.tier.label,
          files: result.files.length,
          matches: result.matches,
        },
        null,
        2,
      )}\n`,
    );
  }
  return 0;
}

if (process.argv[1] && path.resolve(process.argv[1]) === path.resolve(new URL(import.meta.url).pathname)) {
  try {
    process.exit(main(process.argv.slice(2)));
  } catch (err) {
    process.stderr.write(`classify-change: ${err.message}\n`);
    process.exit(1);
  }
}