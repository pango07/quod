// Sample test runs (test-level replay) for the fixture adapter: the sample
// team's quodci/web runs vitest in two packages. Recent commits run on
// quod-action v2: the developer's test job records inline (reported as kind
// "shadow", with no "critical" partner) and runs a few skipped files anyway
// to check them. Older commits, from before the upgrade, show Quod's retired
// setup: a check ("critical") and, for the same commit, a background
// recording job ("shadow"). Shaped on measured runs, not real. Times are
// measured back from now.

import type { RanFile, TestFile, TestFileState, TestRunDetail, TestRunLite, TestRunSummary, TestStats, TestSuite } from "./types";

const REPO = { repoSlug: "web", repository: "quodci/web" } as const;
const MIN = 60_000;

/** The repo-root (".") suite of the sample's other repository. */
const ROOT_REPO = { repoSlug: "quod", repository: "quodci/quod" } as const;
const ROOT_FILES = ["cloud/lib/format.test.ts", "cloud/lib/params.test.ts", "cloud/lib/routes.test.ts", "cloud/lib/trust.test.ts", "cloud/lib/live-events.test.ts", "cloud/components/connect.test.ts", "action/src/inputs.test.ts", "action/src/cache-key.test.ts", "action/src/summary.test.ts"];

interface Seed {
  n: number;
  minutesAgo: number;
  repo?: { repoSlug: string; repository: string };
  suite: string;
  branch: string;
  sha: string;
  total: number;
  ran: RanFile[];
  narrowed: number;
  savedMs: number;
  estPlainMs: number;
  wallMs: number;
  exit: number;
  ledger: TestRunSummary["ledger"];
  /** Skipped files run anyway to check (true: same result). */
  audit: [string, boolean][];
  /** From before quod-action v2: a check, plus a background recording job when it spot-checked. */
  legacy?: boolean;
}

const f = (file: string, why: string, durationMs: number, state = "pass"): RanFile => ({ file, project: "", why, durationMs, state });

const SEEDS: Seed[] = [
  {
    n: 4182,
    minutesAgo: 6,
    suite: "packages/backend",
    branch: "feat/invoice-retries",
    sha: "9f3c2a17e4b80d6c51a2f9e03b7d4c8a1e6f2b90",
    total: 184,
    ran: [
      f("src/billing/invoice.test.ts", "changed: src/billing/invoice.ts", 4210),
      f("src/billing/retry.test.ts", "new test file", 1830),
      f("src/billing/webhook.test.ts", "changed: src/billing/invoice.ts", 2960),
      f("src/billing/ledger.test.ts", "changed: src/billing/invoice.ts", 3380),
      f("src/billing/tax.test.ts", "changed: src/billing/money.ts", 1220),
      f("src/billing/money.test.ts", "changed: src/billing/money.ts", 640),
      f("src/api/routes/invoices.test.ts", "changed: src/billing/invoice.ts", 5120),
      f("src/api/routes/refunds.test.ts", "changed: src/billing/money.ts", 2410),
      f("src/jobs/dunning.test.ts", "changed: src/jobs/dunning.ts", 6830),
      f("src/jobs/scheduler.test.ts", "changed: package.json", 1510),
      f("src/db/migrations.test.ts", "changed: package.json", 8740),
      f("src/lib/config.test.ts", "changed: vitest.config.ts", 410),
      f("src/lib/time.test.ts", "first run", 380),
    ],
    narrowed: 9,
    savedMs: 79_240,
    estPlainMs: 118_900,
    wallMs: 21_400,
    exit: 0,
    ledger: { found: true, sha: "1d7e5b2c90a4f3e8b6c1d2a7e9f0b3c4d5e6f7a8", branch: "main", scope: "main" },
    audit: [
      ["src/api/routes/customers.test.ts", true],
      ["src/billing/discounts.test.ts", true],
      ["src/auth/session.test.ts", true],
      ["src/jobs/export.test.ts", true],
    ],
  },
  {
    n: 4181,
    minutesAgo: 6,
    suite: "packages/web",
    branch: "feat/invoice-retries",
    sha: "9f3c2a17e4b80d6c51a2f9e03b7d4c8a1e6f2b90",
    total: 96,
    ran: [f("src/components/invoice-row.test.tsx", "changed: src/components/invoice-row.tsx", 910)],
    narrowed: 2,
    savedMs: 38_600,
    estPlainMs: 39_510,
    wallMs: 6_200,
    exit: 0,
    ledger: { found: true, sha: "1d7e5b2c90a4f3e8b6c1d2a7e9f0b3c4d5e6f7a8", branch: "main", scope: "main" },
    audit: [
      ["src/pages/settings.test.tsx", true],
      ["src/hooks/use-session.test.ts", true],
    ],
  },
  {
    n: 4175,
    minutesAgo: 95,
    suite: "packages/backend",
    branch: "main",
    sha: "1d7e5b2c90a4f3e8b6c1d2a7e9f0b3c4d5e6f7a8",
    total: 183,
    ran: [
      f("src/auth/session.test.ts", "changed: src/auth/session.ts", 2140),
      f("src/auth/oauth.test.ts", "changed: src/auth/session.ts", 3020),
      f("src/api/routes/login.test.ts", "changed: src/auth/session.ts", 2650, "fail"),
    ],
    narrowed: 4,
    savedMs: 107_300,
    estPlainMs: 115_100,
    wallMs: 14_900,
    exit: 1,
    ledger: { found: true, sha: "c4b9a1e07d3f2865b0e1a9c7d4f3b2a1908e7d6c", branch: "main", scope: "main" },
    audit: [],
  },
  {
    n: 4160,
    minutesAgo: 60 * 26,
    suite: "packages/backend",
    branch: "main",
    sha: "c4b9a1e07d3f2865b0e1a9c7d4f3b2a1908e7d6c",
    total: 183,
    ran: [f("src/lib/time.test.ts", "changed: src/lib/time.ts", 380), f("src/jobs/scheduler.test.ts", "changed: src/lib/time.ts", 1510)],
    narrowed: 11,
    savedMs: 112_600,
    estPlainMs: 114_490,
    wallMs: 12_300,
    exit: 0,
    ledger: { found: true, sha: "77e1d0c3b2a9f8e7d6c5b4a3928170f6e5d4c3b2", branch: "main", scope: "main" },
    audit: [
      ["src/billing/invoice.test.ts", true],
      ["src/db/migrations.test.ts", true],
      ["src/api/routes/invoices.test.ts", true],
    ],
  },
  {
    n: 4102,
    minutesAgo: 60 * 24 * 4,
    suite: "packages/backend",
    branch: "main",
    sha: "77e1d0c3b2a9f8e7d6c5b4a3928170f6e5d4c3b2",
    total: 181,
    ran: Array.from({ length: 181 }, (_, i) => f(`src/suite-${String(i + 1).padStart(3, "0")}.test.ts`, "first run", 400 + ((i * 37) % 900))),
    narrowed: 0,
    savedMs: 0,
    estPlainMs: 113_800,
    wallMs: 41_200,
    exit: 0,
    ledger: { found: false, sha: null, branch: null, scope: null },
    audit: [],
  },
];

/**
 * Older checks of both packages over the last few days (after the backend's
 * first recording, 4 days ago): enough for Trust's "median of the last 20
 * runs" and its 7-day totals. Deterministic, so tests can count on them.
 */
function history(): Seed[] {
  const out: Seed[] = [];
  const ledger = { found: true, sha: "77e1d0c3b2a9f8e7d6c5b4a3928170f6e5d4c3b2", branch: "main", scope: "main" };
  const branches = ["main", "feat/csv-export", "fix/session-expiry", "main", "chore/deps", "feat/search-filters"];
  for (let i = 0; i < 22; i++) {
    for (const [k, suite] of (["packages/backend", "packages/web", "packages/contract"] as const).entries()) {
      const total = suite === "packages/backend" ? 183 : suite === "packages/web" ? 96 : 7;
      // Mostly skipping; now and then a wider change runs a lot more.
      const ranN = suite === "packages/contract" ? (i % 7 === 3 ? 3 : i % 3 === 0 ? 1 : 0) : i % 7 === 3 ? Math.round(total * 0.4) : 2 + ((i * 5 + k * 3) % 11);
      const avg = suite === "packages/backend" ? 640 : suite === "packages/web" ? 410 : 260;
      const sha = ((i + 1) * 2654435761 + k * 97).toString(16).padStart(8, "0").repeat(5).slice(0, 40);
      const ran = Array.from({ length: ranN }, (_, j) => f(`src/${suite === "packages/backend" ? "billing" : "components"}/part-${j + 1}.test.ts`, "changed: src/shared.ts", 300 + ((j * 211 + i) % 1400)));
      out.push({
        n: 4000 - i * 3 - k,
        minutesAgo: 140 + i * 250 + k,
        suite,
        branch: branches[i % branches.length],
        sha,
        total,
        ran,
        narrowed: i % 4,
        savedMs: (total - ranN) * avg,
        estPlainMs: total * avg,
        wallMs: 8_000 + ranN * 300,
        exit: 0,
        ledger,
        audit: i % 4 === 0 ? [[`src/lib/check-${i}.test.ts`, true], [`src/api/check-${i}.test.ts`, true]] : [],
        // The oldest runs, from before the upgrade to quod-action v2.
        legacy: i >= 14,
      });
    }
  }
  // The repo-root suite of quodci/quod: it has only run on a feature branch, so nothing is recorded yet.
  for (const [i, minutesAgo] of [38, 60 * 20].entries()) {
    out.push({
      n: 4179 - i,
      minutesAgo,
      repo: ROOT_REPO,
      suite: ".",
      branch: "feat/vitest",
      sha: i ? "5b1e9c0d2a7f3e8b6c4d1a9e0f7b3c2d5e6a8f14" : "e0c7a2f9b1d4e6c8a3f5b7d9e1c2a4f6b8d0e3a5",
      total: ROOT_FILES.length,
      ran: ROOT_FILES.map((file, j) => f(file, "first run", 220 + j * 37)),
      narrowed: 0,
      savedMs: 0,
      estPlainMs: ROOT_FILES.reduce((a, _, j) => a + 220 + j * 37, 0),
      wallMs: 4_100,
      exit: 0,
      ledger: { found: false, sha: null, branch: null, scope: null },
      audit: [],
    });
  }
  return out;
}

const ALL_SEEDS: Seed[] = [...SEEDS, ...history()];

/**
 * One report. "check": the developer's test job (kind "shadow" when it
 * recorded inline, "critical" for a legacy check). "recording": a legacy
 * background recording job. Ids end in "c" for the developer's job, "s" for
 * a background recording.
 */
function summaryOf(s: Seed, role: "check" | "recording", now: number): TestRunSummary {
  const replayed = s.total - s.ran.length;
  const bg = role === "recording";
  const kind = bg || !s.legacy ? "shadow" : "critical";
  const checked = s.audit.length;
  const runId = 18_900_000_000 + s.n;
  return {
    id: `tr_${s.n}${bg ? "s" : "c"}`,
    at: new Date(now - (s.minutesAgo - (bg ? 2 : 0)) * MIN).toISOString(),
    ...(s.repo ?? REPO),
    suite: s.suite,
    kind,
    tool: "vitest",
    workflow: "ci",
    job: bg ? "record" : "test",
    runId,
    runAttempt: 1,
    runNumber: s.n,
    githubUrl: `https://github.com/${(s.repo ?? REPO).repository}/actions/runs/${runId}`,
    branch: s.branch,
    sha: s.sha,
    event: s.branch === "main" ? "push" : "pull_request",
    files: { total: s.total, ran: s.ran.length, replayed, narrowed: s.narrowed },
    // Inline recording adds a little to the job's own time; a background recording ran every file again.
    timing: { wallMs: s.wallMs + (bg ? 9_000 : s.legacy ? 0 : 600), selectMs: 340 + (s.n % 7) * 20, estPlainMs: s.estPlainMs, savedMs: s.savedMs },
    exit: s.exit,
    ledger: s.ledger,
    // A legacy check carries its background recording's spot-checks once that has reported.
    audits: checked ? { checked, mismatched: s.audit.filter(([, ok]) => !ok).length } : null,
  };
}

function detailOf(s: Seed, role: "check" | "recording", now: number): TestRunDetail {
  const own = summaryOf(s, role, now);
  return {
    ...own,
    ranFiles: s.ran,
    auditFiles: s.audit.map(([file, ok]) => ({ file, ok })),
    shadow: s.legacy && role === "check" && s.audit.length ? summaryOf(s, "recording", now) : null,
  };
}

/** Every sample test run, newest first. Only legacy checks with spot-checks have a background recording. */
export function sampleTestRuns(now = Date.now()): TestRunDetail[] {
  const out: TestRunDetail[] = [];
  for (const s of ALL_SEEDS) {
    if (s.legacy && s.audit.length) out.push(detailOf(s, "recording", now));
    out.push(detailOf(s, "check", now));
  }
  return out.sort((a, b) => b.at.localeCompare(a.at));
}

/** As quod-server adds them up: time saved and recent runs from kind "critical" only, spot-checks from the "shadow" reports that ran them. */
export function sampleTestStats(now = Date.now()): TestStats {
  const all = sampleTestRuns(now);
  const runs = all.filter((r) => r.kind === "critical");
  const midnight = new Date(new Date(now).toISOString().slice(0, 10)).getTime();
  const today = runs.filter((r) => Date.parse(r.at) >= midnight);
  const week = runs.filter((r) => Date.parse(r.at) >= now - 7 * 24 * 60 * MIN);
  const sum = (xs: TestRunSummary[], f: (r: TestRunSummary) => number) => xs.reduce((a, r) => a + f(r), 0);
  const filesTotal = sum(week, (r) => r.files.total);
  const filesSkipped = sum(week, (r) => r.files.replayed);
  const recorders = all.filter((r) => r.kind === "shadow");
  const checked = sum(recorders, (r) => r.audits?.checked ?? 0);
  const wrong = sum(recorders, (r) => r.audits?.mismatched ?? 0);
  return {
    asOf: new Date(now).toISOString(),
    today: { savedMs: sum(today, (r) => r.timing.savedMs ?? 0), runs: today.length },
    week: { savedMs: sum(week, (r) => r.timing.savedMs ?? 0), runs: week.length, filesTotal, filesSkipped, skippedPct: filesTotal ? (filesSkipped / filesTotal) * 100 : null },
    wrongResults: wrong,
    audits: { checked, passed: checked - wrong },
    recent: runs.slice(0, 10).map(summaryOnly),
  };
}

/** A detail without its lists, as the list endpoints send it. */
export function summaryOnly(d: TestRunDetail): TestRunSummary {
  const s: Partial<TestRunDetail> = { ...d };
  delete s.ranFiles;
  delete s.auditFiles;
  delete s.shadow;
  return s as TestRunSummary;
}

// ---------------------------------------------------------------- Test files per suite (Trust)

const BACKEND_DIRS = ["src/api/routes", "src/billing", "src/auth", "src/jobs", "src/db", "src/lib", "src/notifications", "src/search"];
const WEB_DIRS = ["src/components", "src/pages", "src/hooks", "src/lib"];

type FileSeed = Pick<TestFile, "file" | "state" | "reason" | "lastResult"> & { skipped?: number; ran?: number; durationMs?: number | null };

function testFile(s: FileSeed, i: number): TestFile {
  const durationMs = s.durationMs === undefined ? 300 + ((i * 53) % 4200) : s.durationMs;
  const skipped = s.skipped ?? (s.state === "skipping" ? 1 + ((i * 7) % 23) : 0);
  const ran = s.ran ?? (s.state === "learning" ? 0 : 1 + (i % 3));
  return { file: s.file, project: "", state: s.state, reason: s.reason, lastResult: s.lastResult, skipped, ran, durationMs, savedMs: durationMs === null ? null : skipped * durationMs };
}

const STATE_ORDER: Record<TestFileState, number> = { paused: 0, always: 1, learning: 2, skipping: 3, ready: 4 };

function suiteOf(suite: string, ledger: TestSuite["ledger"], special: FileSeed[], dirs: string[], skipping: number, ready: number, ext: string, repo: { repoSlug: string; repository: string } = REPO): TestSuite {
  const seeds: FileSeed[] = [...special];
  const taken = new Set(special.map((s) => s.file));
  let n = 0;
  const next = () => {
    let f: string;
    do {
      f = `${dirs[n % dirs.length]}/module-${String(Math.floor(n / dirs.length) + 1).padStart(2, "0")}.test.${ext}`;
      n++;
    } while (taken.has(f));
    return f;
  };
  for (let i = 0; i < skipping; i++) seeds.push({ file: next(), state: "skipping", reason: null, lastResult: "pass" });
  for (let i = 0; i < ready; i++) seeds.push({ file: next(), state: "ready", reason: null, lastResult: "pass" });
  const files = seeds.map(testFile).sort((a, b) => STATE_ORDER[a.state] - STATE_ORDER[b.state] || a.file.localeCompare(b.file));
  const count = (s: TestFileState) => files.filter((f) => f.state === s).length;
  return {
    ...repo,
    suite,
    ledger,
    totals: {
      files: files.length,
      learning: count("learning"),
      ready: count("ready"),
      skipping: count("skipping"),
      always: count("always"),
      paused: count("paused"),
      // Always-runs files quarantined because their recording behaved differently from the check (newer servers send this).
      recordDiffers: files.filter((f) => f.state === "always" && f.reason === "recording changed its behaviour").length,
      savedMs: files.reduce((a, f) => a + (f.savedMs ?? 0), 0),
    },
    files,
  };
}

/** The sample team's suites: the default branch's newest recording, read per test file. */
export function sampleTestSuites(now = Date.now()): TestSuite[] {
  const ledger = { at: new Date(now - 93 * MIN).toISOString(), sha: "1d7e5b2c90a4f3e8b6c1d2a7e9f0b3c4d5e6f7a8", branch: "main", scope: "main" };
  return [
    suiteOf(
      "packages/backend",
      ledger,
      [
        { file: "src/billing/discounts.test.ts", state: "paused", reason: "a check differed", lastResult: "pass", skipped: 6, ran: 3, durationMs: 2210 },
        { file: "src/api/routes/login.test.ts", state: "always", reason: "it failed last time", lastResult: "fail", skipped: 0, ran: 4, durationMs: 2650 },
        { file: "src/db/migrations.test.ts", state: "always", reason: "recording changed its behaviour", lastResult: "pass", skipped: 0, ran: 5, durationMs: 8740 },
        { file: "src/billing/retry.test.ts", state: "learning", reason: "not recorded yet", lastResult: null, skipped: 0, ran: 0, durationMs: null },
      ],
      BACKEND_DIRS,
      170,
      10,
      "ts",
    ),
    suiteOf(
      "packages/web",
      ledger,
      [{ file: "src/components/date-picker.test.tsx", state: "always", reason: "uses the network", lastResult: "pass", skipped: 0, ran: 3, durationMs: 1340 }],
      WEB_DIRS,
      88,
      7,
      "tsx",
    ),
    // Recorded by the other writer branch.
    suiteOf("packages/contract", { at: new Date(now - 6 * 60 * MIN).toISOString(), sha: "a93d0e7c51b2f4a6e8c0d2b4f6a8c0e2d4f6a8b0", branch: "dev", scope: "dev" }, [], ["src/schema"], 7, 0, "ts"),
    // Only run on a feature branch so far: nothing recorded, every file learning.
    suiteOf(
      ".",
      null,
      ROOT_FILES.map((file) => ({ file, state: "learning" as const, reason: "not recorded yet", lastResult: null, skipped: 0, ran: 0, durationMs: null })),
      [],
      0,
      0,
      "ts",
      ROOT_REPO,
    ),
  ];
}

/**
 * The sample team's test history for the Overview's savings (GET
 * /test-history): about four weeks of CI on quodci/web, from the day Quod
 * was added. Each CI run checks both packages. For the first two weeks the
 * team ran Quod's older setup: pushes to main also recorded them in a
 * background job (kind "shadow" next to the check's "critical"). Since the
 * upgrade to quod-action v2 every test job records inline and is reported
 * as "shadow" alone. Before Quod has a recording, every file runs; after,
 * most are skipped. Deterministic for a given `now`; made up, shaped on
 * measured runs.
 */
export function sampleTestHistory(now = Date.now(), days = 90): TestRunLite[] {
  const DAY = 24 * 60 * MIN;
  const adopted = 26;
  /** Days ago the team moved to quod-action v2 (inline recording). */
  const upgraded = 12;
  const suites = [
    { suite: "packages/backend", total: 184, wallMs: 128_000, fileMs: 118_900 },
    { suite: "packages/web", total: 96, wallMs: 44_000, fileMs: 39_500 },
  ];
  // A tiny deterministic generator, so the sample looks the same on every render.
  let seed = 7;
  const rnd = () => ((seed = (seed * 16807) % 2147483647) - 1) / 2147483646;
  const out: TestRunLite[] = [];
  let n = 9000;
  for (let d = adopted; d >= 0; d--) {
    const perDay = d % 7 === 5 || d % 7 === 6 ? 1 : 3 + Math.floor(rnd() * 3);
    for (let k = 0; k < perDay; k++) {
      const at = now - d * DAY - (k * 3 + 1) * 60 * MIN - Math.floor(rnd() * 50) * MIN;
      if (at > now - 30 * MIN) continue;
      const kind = k % 3;
      const event = kind === 0 ? "pull_request" : "push";
      const branch = kind === 0 ? `refs/pull/${300 + n % 97}/merge` : kind === 1 ? "feat/invoice-retries" : "main";
      const runId = 17_000_000 + n;
      const sha = (n * 2654435761).toString(16).padStart(8, "0").repeat(5);
      const fresh = d >= adopted - 1;
      const inline = d <= upgraded;
      for (const s of suites) {
        const changed = fresh ? s.total : Math.max(1, Math.round(s.total * (0.02 + rnd() * (kind === 2 ? 0.25 : 0.12))));
        const replayed = s.total - changed;
        const share = changed / s.total;
        const ranFileMs = s.fileMs * share;
        const base = { repoSlug: "web", repository: "quodci/web", suite: s.suite, event, branch, runId, sha };
        const wallMs = fresh ? s.wallMs * (0.95 + rnd() * 0.1) : 2_500 + s.wallMs * share * 1.1;
        out.push({
          ...base,
          id: String(n++),
          at: new Date(at).toISOString(),
          // Inline: the test job records the files it ran (a few per cent more time on them).
          kind: inline ? "shadow" : "critical",
          files: { total: s.total, ran: changed, replayed },
          timing: { wallMs: Math.round(inline ? wallMs + ranFileMs * 0.06 : wallMs), estPlainMs: s.fileMs, savedMs: Math.round(s.fileMs - ranFileMs) },
        });
        // Legacy background recordings only on pushes to main (the writer branch), and on the first runs.
        if (!inline && (kind === 2 || fresh)) out.push({
          ...base,
          id: String(n++),
          at: new Date(at + 2 * MIN).toISOString(),
          kind: "shadow",
          files: { total: s.total, ran: s.total, replayed: 0 },
          timing: { wallMs: Math.round(s.wallMs * (0.93 + rnd() * 0.14)), estPlainMs: s.fileMs, savedMs: 0 },
        });
      }
    }
  }
  const since = now - days * DAY;
  return out.filter((r) => Date.parse(r.at) >= since).sort((a, b) => b.at.localeCompare(a.at));
}
