// Sample test runs (test-level replay) for the fixture adapter: the sample
// team's quodci/web runs vitest in two packages. Each commit has the
// developer's check ("critical") and the background recording job
// ("shadow"), which re-runs a few skipped files to check them. Shaped on
// measured runs, not real. Times are measured back from now.

import type { RanFile, TestRunDetail, TestRunSummary, TestStats } from "./types";

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
