#!/usr/bin/env node
// Regenerates the web UI screenshots used in README.md, docs/quickstart.md and the wiki.
//
// Not part of package.json or CI: it needs a browser download and takes about a minute.
//
// What it does, all inside a scratch directory you name:
//   1. makes a scratch project directory and a tokens file with three random tokens (admin, run, read);
//   2. runs a few searches and sweeps on the built-in simulation targets, so the pages have real data;
//   3. starts `hairspring serve` and one remote `hairspring worker` on loopback, on the port you name;
//   4. drives Chromium through the pages (1280x800 desktop, 390x844 phone, light and dark) and writes PNG files;
//   5. stops the two processes it started, by the pids it recorded, and nothing else.
// It never writes to the repository's tracked files. Nothing is written outside the scratch directory except the
// output directory, which defaults to <scratch>/shots. Tokens stay in <scratch>/tokens (mode 0600) and are never
// printed. The script refuses to use a scratch directory inside the repository.
//
// Usage:
//   cargo build -p hairspring-cli                      # a debug build is enough
//   (cd web && npm ci && npm run build)                # the UI is embedded at compile time: build it first
//   mkdir -p /tmp/hs-shots/pw && (cd /tmp/hs-shots/pw && npm init -y && npm i playwright)
//   node scripts/capture-ui-screenshots.mjs --port 7931 --scratch /tmp/hs-shots \
//        [--bin target/debug/hairspring] [--out docs/images/ui] [--playwright-dir /tmp/hs-shots/pw]
//
//   --port        loopback port for the scratch server (required)
//   --scratch     scratch directory (required; created if missing; must be empty (a `pw` or `shots` folder is fine)
//                 or hold a previous run of this script, recognised by its `pids` or `tokens` file; anything else is refused)
//   --bin         the hairspring binary (default: $HAIRSPRING_BIN, else target/debug/hairspring)
//   --out         where the PNG files go (default <scratch>/shots)
//   --playwright-dir  a directory where `npm i playwright` was run (default <scratch>/pw)
//
// The simulation targets have no named properties and Hermit does not run on macOS, so the shots show the honest
// states for them: an empty property table, and a failure that is still a candidate with no checks recorded.
// Populated property tables, the coverage charts and a confirmed failure need the process backend with Hermit on
// Linux x86-64, which this script does not use.
//
// Optional: if `pngquant` or `oxipng` is on the PATH, run it over the output to cut the file sizes.

import { spawn, spawnSync } from 'node:child_process';
import { createRequire } from 'node:module';
import { randomBytes } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const MAX_WAIT_MS = 30_000;

function parseArgs(argv) {
  const out = {};
  for (let i = 0; i < argv.length; i += 2) {
    const key = argv[i];
    if (!key.startsWith('--') || argv[i + 1] === undefined) fail(`bad argument near "${key}"; see the header of this file`);
    out[key.slice(2)] = argv[i + 1];
  }
  return out;
}

const children = [];

function fail(message) {
  console.error(`capture-ui-screenshots: ${message}`);
  stopAll();
  process.exit(2);
}

const args = parseArgs(process.argv.slice(2));
if (!args.port || !/^\d{2,5}$/.test(args.port)) fail('--port is required (a loopback port number)');
if (!args.scratch) fail('--scratch is required');

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const scratch = path.resolve(args.scratch);
if (scratch === repoRoot || scratch.startsWith(repoRoot + path.sep)) fail('--scratch must be outside the repository');
const bin = path.resolve(args.bin ?? process.env.HAIRSPRING_BIN ?? path.join(repoRoot, 'target/debug/hairspring'));
if (!fs.existsSync(bin)) fail(`hairspring binary not found at ${bin}; build it first`);
const outDir = path.resolve(args.out ?? path.join(scratch, 'shots'));
const pwDir = path.resolve(args['playwright-dir'] ?? path.join(scratch, 'pw'));
const port = Number(args.port);
const base = `http://127.0.0.1:${port}`;
const projectDir = path.join(scratch, 'project');

let chromium;
try {
  chromium = createRequire(path.join(pwDir, 'package.json'))('playwright').chromium;
} catch {
  fail(`the playwright package was not found in ${pwDir}; run \`npm i playwright\` there`);
}

// Only a directory this script made before (it holds `pids` or `tokens`), or an empty one, may be reused: the
// project directory inside it is deleted and rebuilt.
if (fs.existsSync(scratch)) {
  const entries = fs.readdirSync(scratch).filter((n) => n !== 'pw' && n !== 'shots');
  const isMine = entries.includes('pids') || entries.includes('tokens');
  if (entries.length > 0 && !isMine) fail(`${scratch} is not empty and was not made by this script (no pids or tokens file); use a new directory`);
}
fs.mkdirSync(scratch, { recursive: true });
fs.rmSync(projectDir, { recursive: true, force: true });
fs.rmSync(path.join(scratch, 'pids'), { force: true });
fs.mkdirSync(projectDir, { recursive: true });
fs.mkdirSync(outDir, { recursive: true });

const tokens = { admin: randomBytes(16).toString('hex'), run: randomBytes(16).toString('hex'), read: randomBytes(16).toString('hex') };
const tokensFile = path.join(scratch, 'tokens');
fs.writeFileSync(tokensFile, Object.entries(tokens).map(([role, t]) => `${t}:${role}`).join('\n') + '\n', { mode: 0o600 });
const runTokenFile = path.join(scratch, 'run-token');
fs.writeFileSync(runTokenFile, tokens.run + '\n', { mode: 0o600 });

/** Runs the CLI in the scratch project. Exit status 1 means "a violation was found", which is what these runs want. */
function cli(...cliArgs) {
  const r = spawnSync(bin, cliArgs, { cwd: projectDir, encoding: 'utf8' });
  if (r.status !== 0 && r.status !== 1) fail(`hairspring ${cliArgs.join(' ')} exited ${r.status}: ${(r.stderr || '').slice(0, 300)}`);
}

function seedData() {
  cli('test', 'pubsub-buggy', '--seed', '5');
  cli('test', 'pubsub-buggy', '--sweep', '--trials', '20', '--seed', '1');
  cli('test', 'lock-lease', '--seed', '7');
  cli('test', 'pubsub-atomic', '--sweep', '--trials', '10', '--seed', '1', '--budget-steps', '2000000');
}

function start(name, cmdArgs) {
  const logFd = fs.openSync(path.join(scratch, `${name}.log`), 'w');
  const child = spawn(bin, cmdArgs, { cwd: projectDir, stdio: ['ignore', logFd, logFd] });
  children.push({ name, child });
  fs.appendFileSync(path.join(scratch, 'pids'), `${name} ${child.pid}\n`);
  return child;
}

function stopAll() {
  for (const { child } of children) {
    try {
      child.kill('SIGTERM');
    } catch {
      // already gone
    }
  }
}

async function waitForServer() {
  const deadline = Date.now() + MAX_WAIT_MS;
  while (Date.now() < deadline) {
    try {
      if ((await fetch(`${base}/api/health`)).ok) return;
    } catch {
      // not up yet
    }
    await new Promise((r) => setTimeout(r, 300));
  }
  fail(`the server did not answer on ${base}; see ${path.join(scratch, 'server.log')}`);
}

const written = [];
const MAX_PAGE_HEIGHT = 1400;

/**
 * Takes one screenshot. By default the shot is the whole page down to the end of its main content, at most
 * MAX_PAGE_HEIGHT pixels tall; `options.screenshot` (for example a `clip`) replaces that. `options.keepFocus` leaves
 * the focused element alone, for a shot of an open search list.
 */
async function shot(page, name, options = {}) {
  await page.mouse.move(0, 0);
  if (!options.keepFocus) await page.evaluate(() => document.activeElement instanceof HTMLElement && document.activeElement.blur());
  await page.waitForTimeout(options.settle ?? 900);
  const width = page.viewportSize().width;
  const contentBottom = await page.evaluate(() => {
    const main = document.querySelector('main');
    return Math.ceil((main ? main.getBoundingClientRect().bottom : document.body.scrollHeight) + window.scrollY + 32);
  });
  const height = Math.min(MAX_PAGE_HEIGHT, Math.max(contentBottom, page.viewportSize().height));
  const file = path.join(outDir, `${name}.png`);
  const defaults = { fullPage: true, clip: { x: 0, y: 0, width, height } };
  await page.screenshot({ path: file, ...(options.screenshot ?? defaults) });
  written.push(file);
}

async function api(path_, token) {
  const r = await fetch(base + path_, { headers: { authorization: `Bearer ${token}` } });
  if (!r.ok) fail(`GET ${path_} answered ${r.status}`);
  return r.json();
}

async function newPage(browser, { token, theme, phone }) {
  const context = await browser.newContext({
    viewport: phone ? { width: 390, height: 844 } : { width: 1280, height: 800 },
    deviceScaleFactor: 1,
    locale: 'en-GB',
    timezoneId: 'UTC',
    colorScheme: theme,
    reducedMotion: 'reduce',
  });
  await context.addInitScript((t) => sessionStorage.setItem('hairspring.token', t), token);
  const page = await context.newPage();
  await page.goto(base + '/');
  await page.getByRole('button', { name: theme === 'dark' ? 'Dark' : 'Light', exact: true }).click();
  return { page, context };
}

async function go(page, route, readyText) {
  await page.goto(base + route);
  if (readyText) await page.getByText(readyText).first().waitFor({ timeout: MAX_WAIT_MS });
}

async function capturePages(browser, theme, ids) {
  const { page, context } = await newPage(browser, { token: tokens.admin, theme });
  const t = theme;
  await go(page, '/', 'Needs attention');
  await shot(page, `overview-${t}`);
  if (t === 'dark') await shot(page, 'theme-switch-dark', { screenshot: { clip: { x: 0, y: 0, width: 1280, height: 120 } } });

  await go(page, '/runs', 'Budget the server will enforce');
  await page.locator('form.start-run select').selectOption('pubsub-atomic');
  await page.locator('form.start-run').getByLabel('Measure how often').check();
  await page.locator('form.start-run').getByLabel(/^Trials/).fill('5');
  await shot(page, `runs-start-form-${t}`);

  await go(page, `/runs/${ids.buggySweep}`, 'What this run shows');
  await shot(page, `run-properties-${t}`, {});

  await go(page, `/failures/${ids.failure}`, 'Candidate');
  await shot(page, `failure-confirmation-${t}`);

  if (t === 'dark') {
    await context.close();
    return;
  }

  await go(page, '/runs?state=failed', 'run_');
  await shot(page, 'runs-filtered-light');
  await go(page, `/runs/${ids.buggySweep}?tab=failures`, 'What this run shows');
  await shot(page, 'run-failures-tab-light');
  await go(page, '/failures', 'fail_');
  await page.getByLabel('State').selectOption('Candidate').catch(() => {});
  await shot(page, 'failures-list-light');
  await go(page, `/runs/${ids.buggySweep}/trials/0`, 'Trial');
  await shot(page, 'trial-light', {});
  await go(page, `/runs/${ids.buggySweep}/seeds`, 'Judged trials');
  await shot(page, 'seeds-explorer-light');
  await go(page, '/targets', 'lock-lease');
  await shot(page, 'targets-light');
  await go(page, '/glossary', 'Bundle');
  await shot(page, 'glossary-light');

  await go(page, `/failures/${ids.failure}`, 'Candidate');
  await page.getByRole('button', { name: 'Confirmation', exact: true }).click();
  await shot(page, 'failure-term-popover-light', { keepFocus: true, screenshot: { clip: { x: 200, y: 60, width: 1080, height: 560 } } });
  await go(page, `/failures/${ids.failure}`, 'Candidate');
  await page.getByRole('button', { name: 'Replay', exact: true }).click();
  await page.getByText(/Failed again on replay|Did not fail on replay/).first().waitFor({ timeout: MAX_WAIT_MS });
  await shot(page, 'failure-replay-light');

  await go(page, '/', 'Needs attention');
  await page.getByPlaceholder(/Run, failure, target/).fill('run_');
  await page.waitForTimeout(1500); // the box waits for a pause in typing before it asks the server
  await shot(page, 'search-results-light', { keepFocus: true, screenshot: { clip: { x: 0, y: 0, width: 1280, height: 520 } } });

  await context.close();
}

/** Starts a sweep from the Runs form as a person would, waits for it to end, and shows the list with it in. */
async function captureStartedFromForm(browser) {
  const { page, context } = await newPage(browser, { token: tokens.admin, theme: 'light' });
  await go(page, '/runs', 'Budget the server will enforce');
  await page.locator('form.start-run select').selectOption('lock-lease');
  await page.locator('form.start-run').getByLabel('Measure how often').check();
  await page.locator('form.start-run').getByLabel(/^Trials/).fill('30');
  await page.getByRole('button', { name: /^Start sweep$/ }).click();
  await page.waitForURL(/\/runs\/run_/, { timeout: MAX_WAIT_MS });
  const runId = new URL(page.url()).pathname.split('/').pop();
  const deadline = Date.now() + MAX_WAIT_MS * 2;
  for (;;) {
    const body = await api(`/api/runs/${runId}`, tokens.admin);
    const state = body.state ?? body.run?.state;
    if (state === undefined) fail(`GET /api/runs/${runId} did not report a state`);
    if (!['queued', 'running'].includes(state)) break;
    if (Date.now() > deadline) fail('the sweep started from the form did not finish in time');
    await page.waitForTimeout(500);
  }
  await go(page, '/runs', 'run_');
  await shot(page, 'runs-list-light');
  await context.close();
}

async function captureReadOnly(browser) {
  const { page, context } = await newPage(browser, { token: tokens.read, theme: 'light' });
  await go(page, '/runs', 'Budget the server will enforce');
  await page.getByText(/cannot start runs/).first().waitFor({ timeout: MAX_WAIT_MS });
  await shot(page, 'runs-read-only-light');
  await go(page, '/workers', 'Queue:');
  await page.getByText('embedded-0').first().waitFor({ timeout: MAX_WAIT_MS });
  await page.getByText('remote-1').first().waitFor({ timeout: MAX_WAIT_MS });
  await shot(page, 'workers-light');
  await context.close();
}

async function capturePhone(browser, theme, runId) {
  const { page, context } = await newPage(browser, { token: tokens.admin, theme, phone: true });
  await go(page, `/runs/${runId}`, 'What this run shows');
  await shot(page, `run-phone-${theme}`);
  await context.close();
}

let exitCode = 0;
try {
  seedData();
  start('server', ['serve', '--addr', `127.0.0.1:${port}`, '--tokens-file', tokensFile, '--organisation', 'Example Ltd']);
  await waitForServer();
  start('worker', ['worker', '--server', base, '--token-file', runTokenFile, '--name', 'remote-1', '--db', path.join(scratch, 'worker.db')]);

  const runs = (await api('/api/runs', tokens.read)).runs;
  const buggySweep = runs.find((r) => r.mode === 'sweep' && r.target === 'pubsub-buggy');
  const failures = (await api('/api/failures', tokens.read)).failures;
  // The failure of the first search: pubsub-buggy at seed 5, shrunk from 23 to 8 steps.
  const failure = failures.find((f) => f.target === 'pubsub-buggy' && f.original_len === 23);
  if (!buggySweep || !failure) fail('the seeded data is missing a sweep or a failure');
  const ids = { buggySweep: buggySweep.id, failure: failure.id };

  const browser = await chromium.launch();
  try {
    await capturePages(browser, 'light', ids);
    await capturePages(browser, 'dark', ids);
    await captureStartedFromForm(browser);
    await captureReadOnly(browser);
    await capturePhone(browser, 'light', ids.buggySweep);
    await capturePhone(browser, 'dark', ids.buggySweep);
  } finally {
    await browser.close();
  }
  for (const tool of ['pngquant', 'oxipng']) {
    if (spawnSync(tool, ['--version'], { stdio: 'ignore' }).status === 0) console.log(`${tool} is installed: run it over ${outDir}`);
  }
  console.log(`wrote ${written.length} files to ${outDir}`);
} catch (error) {
  console.error(error);
  exitCode = 1;
} finally {
  stopAll();
}
process.exit(exitCode);
