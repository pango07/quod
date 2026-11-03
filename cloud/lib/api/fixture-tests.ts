// Sample test runs (test-level replay) for the fixture adapter: the sample
// team's quodci/web runs vitest in two packages. Each commit has the
// developer's check ("critical") and the background recording job
// ("shadow"), which re-runs a few skipped files to check them. Shaped on
// measured runs, not real. Times are measured back from now.

import type { RanFile, TestFile, TestFileState, TestRunDetail, TestRunSummary, TestStats, TestSuite } from "./types";

const REPO = { repoSlug: "web", repository: "quodci/web" } as const;
const MIN = 60_000;

interface Seed {
  n: number;
  minutesAgo: number;
  suite: "packages/backend" | "packages/web";
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
  /** Files the shadow re-ran to check (true: same result). */
  audit: [string, boolean][];
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

function summaryOf(s: Seed, kind: "critical" | "shadow", now: number): TestRunSummary {
  const replayed = s.total - s.ran.length;
  const shadow = kind === "shadow";
  const checked = s.audit.length;
  return {
    id: `tr_${s.n}${shadow ? "s" : "c"}`,
    at: new Date(now - (s.minutesAgo - (shadow ? 2 : 0)) * MIN).toISOString(),
    ...REPO,
    suite: s.suite,
    kind,
    tool: "vitest",
    workflow: shadow ? "quod-record" : "ci",
    job: shadow ? "record" : "test",
    runId: 18_900_000_000 + s.n * (shadow ? 3 : 1),
    runAttempt: 1,
    runNumber: s.n,
    githubUrl: `https://github.com/quodci/web/actions/runs/${18_900_000_000 + s.n * (shadow ? 3 : 1)}`,
    branch: s.branch,
    sha: s.sha,
    event: s.branch === "main" ? "push" : "pull_request",
    files: { total: s.total, ran: s.ran.length, replayed, narrowed: s.narrowed },
    timing: { wallMs: s.wallMs + (shadow ? 9_000 : 0), selectMs: 340 + (s.n % 7) * 20, estPlainMs: s.estPlainMs, savedMs: s.savedMs },
    exit: s.exit,
    ledger: s.ledger,
    // The critical run carries its shadow's spot-checks once the shadow has reported.
    audits: checked ? { checked, mismatched: s.audit.filter(([, ok]) => !ok).length } : null,
  };
}

function detailOf(s: Seed, kind: "critical" | "shadow", now: number): TestRunDetail {
  const own = summaryOf(s, kind, now);
  return {
    ...own,
    ranFiles: s.ran,
    auditFiles: s.audit.map(([file, ok]) => ({ file, ok })),
    shadow: kind === "critical" && s.audit.length ? summaryOf(s, "shadow", now) : null,
  };
}

/** Every sample test run, newest first. Only critical runs with spot-checks have a shadow. */
export function sampleTestRuns(now = Date.now()): TestRunDetail[] {
  const out: TestRunDetail[] = [];
  for (const s of SEEDS) {
    if (s.audit.length) out.push(detailOf(s, "shadow", now));
    out.push(detailOf(s, "critical", now));
  }
  return out.sort((a, b) => b.at.localeCompare(a.at));
}

export function sampleTestStats(now = Date.now()): TestStats {
  const runs = sampleTestRuns(now).filter((r) => r.kind === "critical");
  const midnight = new Date(new Date(now).toISOString().slice(0, 10)).getTime();
  const today = runs.filter((r) => Date.parse(r.at) >= midnight);
  const week = runs.filter((r) => Date.parse(r.at) >= now - 7 * 24 * 60 * MIN);
  const sum = (xs: TestRunSummary[], f: (r: TestRunSummary) => number) => xs.reduce((a, r) => a + f(r), 0);
  const filesTotal = sum(week, (r) => r.files.total);
  const filesSkipped = sum(week, (r) => r.files.replayed);
  const checked = sum(runs, (r) => r.audits?.checked ?? 0);
  const wrong = sum(runs, (r) => r.audits?.mismatched ?? 0);
  return {
    asOf: new Date(now).toISOString(),
    today: { savedMs: sum(today, (r) => r.timing.savedMs ?? 0), runs: today.length },
    week: { savedMs: sum(week, (r) => r.timing.savedMs ?? 0), runs: week.length, filesTotal, filesSkipped, skippedPct: filesTotal ? (filesSkipped / filesTotal) * 100 : null },
    wrongResults: wrong,
    audits: { checked, passed: checked - wrong },
    recent: sampleTestRuns(now).slice(0, 10).map(summaryOnly),
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

function suiteOf(suite: string, ledger: TestSuite["ledger"], special: FileSeed[], dirs: string[], skipping: number, ready: number, ext: string): TestSuite {
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
    ...REPO,
    suite,
    ledger,
    totals: {
      files: files.length,
      learning: count("learning"),
      ready: count("ready"),
      skipping: count("skipping"),
      always: count("always"),
      paused: count("paused"),
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
        { file: "src/billing/discounts.test.ts", state: "paused", reason: "a spot-check differed", lastResult: "pass", skipped: 6, ran: 3, durationMs: 2210 },
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
      [{ file: "src/components/date-picker.test.tsx", state: "always", reason: "recording changed its behaviour", lastResult: "pass", skipped: 0, ran: 3, durationMs: 1340 }],
      WEB_DIRS,
      88,
      7,
      "tsx",
    ),
  ];
}
