// Sample data for the fixture adapter, taken from the approved prototypes
// (Main, Repo, Run, Step, Connect .dc.html). Numbers are shaped on measured
// replays but are not real. Only fixture.ts imports this file; pages read
// through the CloudApi.

import type {
  DayMinutes,
  Divergence,
  InputCounts,
  Range,
  RepoSummary,
  RunDetail,
  RunJob,
  RunStep,
  RunSummary,
  RunTimePoint,
  StepSummary,
  StepWhy,
  Team,
  TeamTotals,
  TrustEvent,
  TrustState,
} from "./types";

/**
 * The fixture clock is the real clock: sample times ("4m ago") are measured
 * back from now, so they sit next to the simulator's live runs (lib/live/sim.ts).
 */
export const asOf = () => new Date().toISOString();
const ago = (minutes: number) => new Date(Date.now() - minutes * 60_000).toISOString();

export const USD_PER_MINUTE = 0.008;

/** The demo session's sample team: the only org with sample data and the simulator. Real WorkOS org ids look like "org_01..". */
export const SAMPLE_ORG_ID = "org_demo_quodci";

export const isSampleOrg = (orgId: string | null | undefined): boolean => orgId === SAMPLE_ORG_ID;

export const TEAMS: Team[] = [
  { slug: "quodci", name: "quodci", plan: "Pro", orgId: "org_demo_quodci" },
  { slug: "jvaught", name: "jvaught", plan: "Hobby", orgId: "org_demo_jvaught" },
];

// ---------------------------------------------------------------- Totals (Main.dc.html)

const TOTALS: Record<Range, Omit<TeamTotals, "range" | "costSavedUsd" | "usdPerMinute">> = {
  "7d": { savedMinutes: 1284, stepsReplayed: 3412, stepsTotal: 5590, wrongReplays: 0, spotChecks: 172, spotChecksIdentical: 172 },
  "30d": { savedMinutes: 5290, stepsReplayed: 13904, stepsTotal: 23970, wrongReplays: 0, spotChecks: 698, spotChecksIdentical: 698 },
  "90d": { savedMinutes: 14460, stepsReplayed: 36880, stepsTotal: 68300, wrongReplays: 0, spotChecks: 1840, spotChecksIdentical: 1840 },
};

export function totalsFor(range: Range): TeamTotals {
  const t = TOTALS[range];
  return { range, ...t, usdPerMinute: USD_PER_MINUTE, costSavedUsd: Math.round(t.savedMinutes * USD_PER_MINUTE * 100) / 100 };
}

export const EMPTY_TOTALS = (range: Range): TeamTotals => ({
  range,
  savedMinutes: 0,
  stepsReplayed: 0,
  stepsTotal: 0,
  wrongReplays: 0,
  spotChecks: 0,
  spotChecksIdentical: 0,
  usdPerMinute: USD_PER_MINUTE,
  costSavedUsd: 0,
});

const DAYS_IN: Record<Range, number> = { "7d": 7, "30d": 30, "90d": 90 };
// The prototype's bar heights, per range (replayed) and shared (ran).
const SEEDS: Record<Range, number[]> = {
  "7d": [38, 44, 51, 47, 55, 62, 58, 64, 61, 66, 59, 68, 63, 70],
  "30d": [30, 34, 41, 39, 45, 52, 49, 55, 58, 54, 60, 57, 63, 61],
  "90d": [12, 18, 22, 27, 31, 35, 40, 44, 47, 50, 52, 55, 57, 58],
};
const RAN_SEEDS = [26, 24, 22, 25, 21, 18, 20, 17, 19, 16, 20, 15, 18, 14];

function sample(seeds: number[], n: number): number[] {
  return Array.from({ length: n }, (_, i) => {
    const t = n === 1 ? seeds.length - 1 : (i / (n - 1)) * (seeds.length - 1);
    const lo = Math.floor(t);
    const hi = Math.min(seeds.length - 1, lo + 1);
    const v = seeds[lo] + (seeds[hi] - seeds[lo]) * (t - lo);
    // A deterministic weekday wobble so long ranges do not look like a ramp.
    return Math.max(1, v * (1 + 0.12 * Math.sin(i * 1.7)));
  });
}

/** Spread `total` over weights, in whole numbers that add up exactly. */
function spread(total: number, weights: number[]): number[] {
  const sum = weights.reduce((a, b) => a + b, 0);
  const raw = weights.map((w) => (w / sum) * total);
  const out = raw.map(Math.floor);
  let left = total - out.reduce((a, b) => a + b, 0);
  const order = raw.map((r, i) => [r - Math.floor(r), i] as const).sort((a, b) => b[0] - a[0]);
  for (const [, i] of order) {
    if (left-- <= 0) break;
    out[i] += 1;
  }
  return out;
}

export function daysFor(range: Range): DayMinutes[] {
  const n = DAYS_IN[range];
  const saved = spread(TOTALS[range].savedMinutes, sample(SEEDS[range], n));
  const ran = spread(Math.round(TOTALS[range].savedMinutes * 0.52), sample(RAN_SEEDS, n));
  return saved.map((replayedMinutes, i) => {
    const d = new Date(Date.now() - (n - 1 - i) * 86_400_000);
    return { day: d.toISOString().slice(0, 10), replayedMinutes, ranMinutes: ran[i] };
  });
}

export const RUNNERS = [
  { name: "hetzner-ash-1", kind: "self-hosted", status: "idle" },
  { name: "hetzner-ash-2", kind: "self-hosted", status: "running" },
  { name: "github-hosted", kind: "github-hosted", status: "idle" },
] as const;

export const TRUST_COUNTS = { proven: 38, proving: 6, watching: 3 };

// ---------------------------------------------------------------- Repositories

export interface StepSeed {
  key: string;
  job: string;
  command: string;
  trust: TrustState;
  replayRate: number | null;
  ranSeconds: number;
  replaySeconds: number | null;
  /** Share of the repo's saved time; explicit minutes win. */
  savedMinutes: number;
  recordedInputs: number;
  /** Uses the network: never memoized (shown in runs, not in the steps list). */
  network?: boolean;
}

export interface RepoSeed {
  summary: RepoSummary;
  defaultBranch: string;
  firstRun: number;
  /** Median run without Quod, in seconds. */
  baseSeconds: number;
  /** A file a typical change touches, for the "first change" receipt. */
  changedPath: string;
  steps: StepSeed[];
  runner: string;
  actor: string;
}

const REPLAYING: TrustState = { kind: "replaying" };

export const REPOS: RepoSeed[] = [
  {
    summary: {
      slug: "quod",
      fullName: "quodci/quod",
      language: "Rust",
      runner: "self-hosted",
      replayRate: 0.64,
      savedMinutes: 552,
      get lastRunAt() {
        return ago(4);
      },
      lastRunStatus: "passed",
      pausedSteps: 0,
    },
    defaultBranch: "main",
    firstRun: 1819,
    baseSeconds: 391,
    changedPath: "crates/quod-hooks/src/job.rs",
    runner: "hetzner-ash-2",
    actor: "jvaught",
    steps: [
      { key: "4c1e9a07", job: "check", command: "cargo fmt --all --check", trust: REPLAYING, replayRate: 0.96, ranSeconds: 38.4, replaySeconds: 0.21, savedMinutes: 161, recordedInputs: 412 },
      { key: "b81f03d2", job: "check", command: "cargo clippy --workspace", trust: REPLAYING, replayRate: 0.91, ranSeconds: 129, replaySeconds: 0.34, savedMinutes: 232, recordedInputs: 3108 },
      { key: "e5a7c914", job: "check", command: "cargo test --workspace", trust: REPLAYING, replayRate: 0.42, ranSeconds: 200, replaySeconds: 0.4, savedMinutes: 131, recordedInputs: 3421 },
      { key: "1d90b6ef", job: "web", command: "npm ci", trust: REPLAYING, replayRate: 0.9, ranSeconds: 22, replaySeconds: 0.19, savedMinutes: 15, recordedInputs: 1904 },
      { key: "93c2e85a", job: "web", command: "npm run build", trust: REPLAYING, replayRate: 0.88, ranSeconds: 46, replaySeconds: 0.28, savedMinutes: 28, recordedInputs: 2671 },
      { key: "6f4d21b0", job: "integration", command: "cargo test --test acceptance", trust: { kind: "proving", have: 1, need: 2 }, replayRate: 0, ranSeconds: 242, replaySeconds: null, savedMinutes: 0, recordedInputs: 3980 },
      { key: "c07e5d13", job: "release", command: "cargo build --release", trust: { kind: "watching", wouldHaveReplayed: 9 }, replayRate: null, ranSeconds: 344, replaySeconds: null, savedMinutes: 0, recordedInputs: 2207 },
    ],
  },
  {
    summary: {
      slug: "web",
      fullName: "quodci/web",
      language: "Node",
      runner: "github-hosted",
      replayRate: 0.71,
      savedMinutes: 340,
      get lastRunAt() {
        return ago(22);
      },
      lastRunStatus: "passed",
      pausedSteps: 0,
    },
    defaultBranch: "main",
    firstRun: 612,
    baseSeconds: 174,
    changedPath: "app/page.tsx",
    runner: "github-hosted",
    actor: "jvaught",
    steps: [
      { key: "a3d5f017", job: "build", command: "npm ci", trust: REPLAYING, replayRate: 0.93, ranSeconds: 24, replaySeconds: 0.2, savedMinutes: 58, recordedInputs: 2210 },
      { key: "5be0c2a9", job: "build", command: "npm run lint", trust: REPLAYING, replayRate: 0.74, ranSeconds: 31, replaySeconds: 0.22, savedMinutes: 66, recordedInputs: 1187 },
      { key: "d4a19e36", job: "build", command: "tsc --noEmit", trust: REPLAYING, replayRate: 0.72, ranSeconds: 27, replaySeconds: 0.18, savedMinutes: 55, recordedInputs: 1302 },
      { key: "08f7b2c4", job: "build", command: "next build", trust: REPLAYING, replayRate: 0.69, ranSeconds: 74, replaySeconds: 0.31, savedMinutes: 161, recordedInputs: 2968 },
    ],
  },
  {
    summary: {
      slug: "payments",
      fullName: "acme/payments",
      language: "Go",
      runner: "self-hosted",
      replayRate: 0.52,
      savedMinutes: 298,
      get lastRunAt() {
        return ago(58);
      },
      lastRunStatus: "passed",
      pausedSteps: 1,
    },
    defaultBranch: "main",
    firstRun: 985,
    baseSeconds: 312,
    changedPath: "internal/coupon/apply.go",
    runner: "hetzner-ash-1",
    actor: "mrivera",
    steps: [
      { key: "2e6b0d71", job: "test", command: "go vet ./...", trust: REPLAYING, replayRate: 0.9, ranSeconds: 19, replaySeconds: 0.17, savedMinutes: 41, recordedInputs: 1630 },
      { key: "f19c47a8", job: "test", command: "go test ./...", trust: REPLAYING, replayRate: 0.58, ranSeconds: 96, replaySeconds: 0.36, savedMinutes: 85, recordedInputs: 2402 },
      { key: "7be2d410", job: "e2e", command: "npx playwright test", trust: { kind: "paused", consistent: 1, need: 3 }, replayRate: 0.47, ranSeconds: 251, replaySeconds: 0.33, savedMinutes: 172, recordedInputs: 2914 },
    ],
  },
  {
    summary: {
      slug: "ml-pipeline",
      fullName: "acme/ml-pipeline",
      language: "Python",
      runner: "github-hosted",
      replayRate: 0.39,
      savedMinutes: 94,
      get lastRunAt() {
        return ago(124);
      },
      lastRunStatus: "passed",
      pausedSteps: 0,
    },
    defaultBranch: "main",
    firstRun: 301,
    baseSeconds: 506,
    changedPath: "pipeline/features.py",
    runner: "github-hosted",
    actor: "aokafor",
    steps: [
      { key: "39d8a6c2", job: "lint", command: "ruff check .", trust: REPLAYING, replayRate: 0.81, ranSeconds: 9, replaySeconds: 0.14, savedMinutes: 12, recordedInputs: 388 },
      { key: "c5e2f9b7", job: "lint", command: "mypy pipeline", trust: REPLAYING, replayRate: 0.63, ranSeconds: 44, replaySeconds: 0.26, savedMinutes: 30, recordedInputs: 941 },
      { key: "8a1f6d03", job: "test", command: "pytest -q", trust: REPLAYING, replayRate: 0.31, ranSeconds: 162, replaySeconds: 0.38, savedMinutes: 52, recordedInputs: 1755 },
      { key: "e70b3c95", job: "test", command: "python -m pipeline.smoke", trust: { kind: "proving", have: 1, need: 2 }, replayRate: 0, ranSeconds: 211, replaySeconds: null, savedMinutes: 0, recordedInputs: 1209 },
    ],
  },
];

export function stepSummary(repo: RepoSeed, s: StepSeed, trust: TrustState = s.trust): StepSummary {
  return {
    repoSlug: repo.summary.slug,
    key: s.key,
    job: s.job,
    command: s.command,
    trust,
    replayRate: s.replayRate,
    ranSeconds: s.ranSeconds,
    replaySeconds: s.replaySeconds,
    savedMinutes: s.savedMinutes,
  };
}

// ---------------------------------------------------------------- Runs (Repo.dc.html, Run.dc.html)

// Run time as a share of the repo's slowest run, from the prototype's chart:
// "full" is without Quod (recorded), "with" is the share of that with Quod.
const FULL = [92, 88, 95, 90, 86, 93, 97, 89, 91, 94, 88, 90, 96, 87, 92, 95, 89, 93, 90, 94, 88, 91, 95, 92];
const WITH = [96, 22, 18, 85, 20, 17, 92, 19, 21, 16, 88, 18, 20, 15, 17, 90, 19, 16, 18, 21, 17, 86, 15, 16];
export const RUNS_PER_REPO = FULL.length;

export const TITLES = [
  "web: plain-language copy a junior dev can follow",
  "docs: explain the trust meter",
  "hooks: keep step output when the runner restarts",
  "ci: cache the toolchain download",
  "fix flaky timeout in the coupon test",
  "README: one-line install",
  "bump dependencies",
  "store: compress large outputs",
  "observer: count directory listings",
  "refactor: split the receipt renderer",
  "docs: typo",
  "add a smoke test for replay",
];

function sha(n: number, salt: string): string {
  let a = 0x811c9dc5 ^ n;
  let b = 0x01000193;
  for (const c of salt + n) {
    a = Math.imul(a ^ c.charCodeAt(0), 16777619) >>> 0;
    b = Math.imul(b ^ c.charCodeAt(0), 2246822519) >>> 0;
  }
  return (a.toString(16).padStart(8, "0") + b.toString(16).padStart(8, "0") + "0".repeat(24)).slice(0, 40);
}

const hash8 = (n: number, salt: string) => `b3:${sha(n * 31 + 7, salt).slice(0, 8)}`;

export function runTimes(repo: RepoSeed): RunTimePoint[] {
  const scale = repo.baseSeconds / 91.5;
  return FULL.map((f, i) => {
    const number = repo.firstRun + i;
    if (repo.summary.slug === "quod" && i === FULL.length - 1) return { number, withQuodSeconds: 62, withoutSeconds: 400 };
    const without = Math.round(f * scale);
    return { number, withQuodSeconds: Math.max(8, Math.round((without * WITH[i]) / 100)), withoutSeconds: without };
  });
}

/** Run #1842 on quodci/quod, exactly as Run.dc.html shows it. */
function run1842(repo: RepoSeed): RunDetail {
  const [fmt, clippy, test, npmci, build] = repo.steps;
  const replayed = (s: StepSeed, seconds: number, savedSeconds: number, sourceRun: number): RunStep => ({
    name: s.job === "web" ? `web: ${s.command}` : s.command,
    outcome: "replayed",
    seconds,
    why: { kind: "replayed", stepKey: s.key, sourceRun, inputsMatched: s.recordedInputs, proof: { have: 2, need: 2 }, divergences: 0, savedSeconds },
  });
  const steps: RunStep[] = [
    { name: "Set up job", outcome: "ran", seconds: 2.9, why: { kind: "not-memoized", what: "runner setup" } },
    { name: "Checkout", outcome: "ran", seconds: 1.1, why: { kind: "not-memoized", what: "checkout" } },
    replayed(fmt, 0.21, 38.4, 1839),
    replayed(clippy, 0.34, 129, 1839),
    {
      name: test.command,
      outcome: "ran",
      seconds: 51.8,
      why: {
        kind: "changed",
        stepKey: test.key,
        firstChange: { path: "crates/quod-hooks/src/job.rs", was: "b3:9a07f2d1", now: "b3:e31c08aa", what: "content" },
        otherChanges: 2,
        recordedInputs: test.recordedInputs,
        proof: { have: 1, need: 2 },
      },
    },
    replayed(npmci, 0.19, 22, 1840),
    replayed(build, 0.28, 46, 1840),
    { name: "upload artifact", outcome: "ran", seconds: 1.4, why: { kind: "network" } },
  ];
  return {
    ...summaryOf(repo, 1842, steps, { withQuodSeconds: 62, withoutSeconds: 400 }, "7c8e644b2d19a0f3c55e81d7a6b4f90e12c3d8a1"),
    githubUrl: "https://github.com/pango07/quod",
    jobs: [{ name: "check / ubuntu-24.04", steps }],
  };
}

function summaryOf(repo: RepoSeed, number: number, steps: RunStep[], time: Omit<RunTimePoint, "number">, commit?: string): RunSummary {
  const i = number - repo.firstRun;
  const lastAgoMin = (Date.now() - Date.parse(repo.summary.lastRunAt ?? asOf())) / 60_000;
  const spot = steps.filter((s) => s.why.kind === "spot-check");
  return {
    repoSlug: repo.summary.slug,
    number,
    title: repo.summary.slug === "quod" && i === RUNS_PER_REPO - 1 ? TITLES[0] : TITLES[(number * 7) % TITLES.length],
    branch: i % 5 === 2 ? "feature/receipts" : repo.defaultBranch,
    sha: commit ?? sha(number, repo.summary.slug),
    actor: repo.actor,
    event: i % 5 === 2 ? "pull_request" : "push",
    runner: repo.runner,
    status: "passed",
    startedAt: ago(lastAgoMin + (RUNS_PER_REPO - 1 - i) * 47),
    tookSeconds: time.withQuodSeconds,
    wouldHaveTakenSeconds: time.withoutSeconds,
    stepsReplayed: steps.filter((s) => s.outcome === "replayed").length,
    stepsTotal: steps.length,
    spotChecks: spot.length,
    spotChecksIdentical: spot.filter((s) => s.why.kind === "spot-check" && s.why.identical).length,
  };
}

/** The payments e2e story from Step.dc.html: proven at #985/#988, diverged on spot check #1003. */
const E2E = { key: "7be2d410", first: 985, proven: 988, spotPassed: 996, diverged: 1003, consistentSince: 1004, consistentRun: 1007 };

type Tagged = { job: string | null; step: RunStep };

function genericSteps(repo: RepoSeed, number: number, need: (s: StepSeed) => number): Tagged[] {
  const i = number - repo.firstRun;
  const heavy = WITH[i] > 50;
  const out: Tagged[] = [
    { job: null, step: { name: "Set up job", outcome: "ran", seconds: 2.4 + (i % 4) * 0.3, why: { kind: "not-memoized", what: "runner setup" } } },
    { job: null, step: { name: "Checkout", outcome: "ran", seconds: 0.9 + (i % 3) * 0.2, why: { kind: "not-memoized", what: "checkout" } } },
  ];
  let spotDone = false;
  for (const [j, s] of repo.steps.entries()) {
    const ran = (why: StepWhy): Tagged => ({ job: s.job, step: { name: s.command, outcome: "ran", seconds: s.ranSeconds * (0.92 + ((i + j) % 5) * 0.04), why } });
    const replayed = (sourceRun: number): Tagged => ({
      job: s.job,
      step: { name: s.command, outcome: "replayed", seconds: s.replaySeconds ?? 0.3, why: { kind: "replayed", stepKey: s.key, sourceRun, inputsMatched: s.recordedInputs, proof: { have: 2, need: 2 }, divergences: 0, savedSeconds: s.ranSeconds } },
    });
    const changed = (have: number, others: number): StepWhy => ({
      kind: "changed",
      stepKey: s.key,
      firstChange: { path: repo.changedPath, was: hash8(number - 1, s.key), now: hash8(number, s.key), what: "content" },
      otherChanges: others,
      recordedInputs: s.recordedInputs,
      proof: { have, need: 2 },
    });

    // The paused e2e step in acme/payments follows its own history.
    if (repo.summary.slug === "payments" && s.key === E2E.key) {
      if (number === E2E.first) out.push(ran({ kind: "first-run", stepKey: s.key, recordedInputs: s.recordedInputs, proof: { have: 1, need: 2 } }));
      else if (number === E2E.proven) out.push(ran(changed(2, 0)));
      else if (number < E2E.proven) out.push(ran(changed(1, 0)));
      else if (number === E2E.spotPassed || number === E2E.diverged)
        out.push(ran({ kind: "spot-check", stepKey: s.key, sourceRun: E2E.proven, inputsMatched: s.recordedInputs, identical: number === E2E.spotPassed }));
      else if (number > E2E.diverged) out.push(ran({ kind: "paused", stepKey: s.key, consistent: number >= E2E.consistentRun ? 1 : 0, need: need(s) }));
      else out.push(replayed(E2E.proven));
      continue;
    }

    if (s.network) out.push(ran({ kind: "network" }));
    else if (s.trust.kind === "watching") out.push(ran({ kind: "watching", stepKey: s.key, couldHaveReplayed: !heavy, recordedInputs: s.recordedInputs }));
    else if (s.trust.kind === "proving") out.push(ran({ ...changed(1, 1 + (i % 3)), proof: { have: 1, need: s.trust.need } } as StepWhy));
    else if (heavy && (s.replayRate ?? 0) < 0.92) out.push(ran(changed(1, (i + j) % 4)));
    else if (!spotDone && (number + j) % 9 === 0) {
      spotDone = true;
      out.push(ran({ kind: "spot-check", stepKey: s.key, sourceRun: number - 1 - (j % 3), inputsMatched: s.recordedInputs, identical: true }));
    } else out.push(replayed(number - 1 - ((i + j) % 3)));
  }
  return out;
}

export function runDetail(repo: RepoSeed, number: number, need: (s: StepSeed) => number): RunDetail | null {
  const i = number - repo.firstRun;
  if (!Number.isInteger(number) || i < 0 || i >= RUNS_PER_REPO) return null;
  if (repo.summary.slug === "quod" && number === 1842) return run1842(repo);
  const tagged = genericSteps(repo, number, need);
  const setup = tagged.filter((t) => t.job === null).map((t) => t.step);
  const jobs: RunJob[] = [...new Set(repo.steps.map((s) => s.job))].map((job) => ({
    name: `${job} / ubuntu-24.04`,
    steps: [...setup, ...tagged.filter((t) => t.job === job).map((t) => t.step)],
  }));
  const all = jobs.flatMap((j) => j.steps);
  return {
    ...summaryOf(repo, number, all, runTimes(repo)[i]),
    githubUrl: `https://github.com/${repo.summary.fullName}/actions`,
    jobs,
  };
}

// ---------------------------------------------------------------- Step trust (Step.dc.html)

export interface TrustSeed {
  divergence: Divergence | null;
  history: TrustEvent[];
  inputs: InputCounts;
}

export function trustSeed(repo: RepoSeed, s: StepSeed): TrustSeed {
  if (repo.summary.slug === "payments" && s.key === E2E.key) {
    return {
      divergence: {
        recordedRun: E2E.proven,
        recheckRun: E2E.diverged,
        inputs: 2914,
        stream: "stdout",
        line: 41,
        recorded: ["  ✓ checkout › applies coupon (2.1s)"],
        recheck: ["  ✘ checkout › applies coupon (timeout 5000ms)"],
        exitCode: { recorded: 0, recheck: 1 },
      },
      history: [
        { kind: "consistent", at: ago(120), run: E2E.consistentRun, sameAs: E2E.consistentSince },
        { kind: "paused", at: ago(60 * 24), run: E2E.diverged },
        { kind: "spot-check-passed", at: ago(60 * 48), run: E2E.spotPassed },
        { kind: "replayed", at: ago(60 * 72), times: 41, savedMinutes: 172 },
        { kind: "proven", at: ago(60 * 120), runs: [E2E.first, E2E.proven] },
        { kind: "first-recorded", at: ago(60 * 121), run: E2E.first, seconds: 251 },
      ],
      inputs: { filesRead: 2511, foldersListed: 342, pathsNotFound: 61, networkAccess: 0 },
    };
  }
  const n = s.recordedInputs;
  const folders = Math.round(n * 0.11);
  const missing = Math.round(n * 0.02);
  const first = repo.firstRun;
  const history: TrustEvent[] = [];
  if (s.trust.kind === "replaying") {
    history.push(
      { kind: "spot-check-passed", at: ago(60 * 5), run: first + 21 },
      { kind: "replayed", at: ago(60 * 26), times: Math.round((s.replayRate ?? 0.5) * 40), savedMinutes: s.savedMinutes },
      { kind: "spot-check-passed", at: ago(60 * 50), run: first + 9 },
      { kind: "proven", at: ago(60 * 118), runs: [first, first + 2] },
      { kind: "first-recorded", at: ago(60 * 120), run: first, seconds: s.ranSeconds },
    );
  } else if (s.trust.kind === "proving") {
    history.push({ kind: "first-recorded", at: ago(60 * 3), run: first + RUNS_PER_REPO - 2, seconds: s.ranSeconds });
  } else {
    history.push({ kind: "first-recorded", at: ago(60 * 120), run: first, seconds: s.ranSeconds });
  }
  return { divergence: null, history, inputs: { filesRead: n - folders - missing, foldersListed: folders, pathsNotFound: missing, networkAccess: 0 } };
}

export const CONNECT = {
  setupAction: "quodci/setup@v1",
  runnerLabels: ["self-hosted", "quod"],
  installCommand: "curl -fsSL https://quodci.com/install.sh | sh",
  selfHostedExample: "https://quod.internal.example",
};
