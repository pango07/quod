// The live simulator behind "Sample data, simulated live".
//
// Every ~20 seconds a CI run starts on one of the sample team's repos. Its jobs
// queue, start, and run their steps; steps replay or run; totals move. Nothing
// is stored: the whole timeline is a pure function of the wall clock and a
// fixed seed, so every server instance (and every reconnect) agrees on what
// happened when, and a client that resumes with Last-Event-ID gets exactly the
// events it missed. Built from the fixture repos and steps (lib/api/fixture-data).

import { REPOS, RUNS_PER_REPO, TITLES, type RepoSeed, type StepSeed } from "@/lib/api/fixture-data";
import { inScope } from "./scope";
import type { LiveEvent, LiveJob, LiveScope, RunDetail, RunStep, RunSummary, StatsDelta, StepWhy, TrustEvent } from "@/lib/api/types";

export const SIM_TEAM = "quodci";
/** The simulator's epoch: cycle 0 starts here. */
export const T0 = Date.UTC(2026, 10, 1);
export const PERIOD_MS = 20_000;
/** No simulated run lasts longer than this (checked by a test). */
export const MAX_RUN_MS = 150_000;

/** Which repo each cycle runs, repeating. quod is the busiest, like a real team's main repo. */
const PATTERN = ["quod", "web", "quod", "payments", "quod", "ml-pipeline", "web", "quod", "payments", "web"];
const ACTORS = ["jvaught", "mrivera", "aokafor", "slee"];

// ---------------------------------------------------------------- Deterministic randomness

/** mulberry32: a small, fast, seeded PRNG. Same seed, same sequence, everywhere. */
function rng(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const hex = (r: () => number, n: number) => Array.from({ length: n }, () => Math.floor(r() * 16).toString(16)).join("");
const between = (r: () => number, lo: number, hi: number) => lo + (hi - lo) * r();
const round2 = (x: number) => Math.round(x * 100) / 100;

// ---------------------------------------------------------------- The plan of one cycle's run

export interface SimStep {
  index: number;
  name: string;
  stepKey: string | null;
  outcome: "replayed" | "ran";
  seconds: number;
  /** What the step would have taken had it run (equal to `seconds` when it ran). */
  fullSeconds: number;
  why: StepWhy;
  startAt: number;
  endAt: number;
}

export interface SimJob {
  id: string;
  name: string;
  queuedAt: number;
  startedAt: number;
  finishedAt: number;
  steps: SimStep[];
}

export interface SimRun {
  cycle: number;
  repo: RepoSeed;
  number: number;
  title: string;
  branch: string;
  sha: string;
  actor: string;
  event: "push" | "pull_request";
  startedAt: number;
  finishedAt: number;
  jobs: SimJob[];
}

const repoBySlug = (slug: string) => REPOS.find((r) => r.summary.slug === slug);
const positions = (slug: string) => PATTERN.flatMap((s, i) => (s === slug ? [i] : []));
/** Simulated run numbers continue after the fixture's last run (quodci/quod #1842 -> #1843...). */
const baseNumber = (repo: RepoSeed) => repo.firstRun + RUNS_PER_REPO;

export const cycleStart = (cycle: number) => T0 + cycle * PERIOD_MS;
export const cycleAt = (t: number) => Math.floor((t - T0) / PERIOD_MS);

function numberFor(cycle: number, repo: RepoSeed): number {
  const pos = positions(repo.summary.slug);
  const block = Math.floor(cycle / PATTERN.length);
  const rank = pos.indexOf(cycle % PATTERN.length);
  return baseNumber(repo) + block * pos.length + rank;
}

/** The cycle that produced a repo's run number, or null if that number is not simulated. */
export function cycleFor(repoSlug: string, number: number): number | null {
  const repo = repoBySlug(repoSlug);
  if (!repo || !Number.isSafeInteger(number)) return null;
  const pos = positions(repoSlug);
  const k = number - baseNumber(repo);
  if (k < 0 || pos.length === 0) return null;
  return Math.floor(k / pos.length) * PATTERN.length + pos[k % pos.length];
}

/** Which jobs a run of this repo has: grouped by the fixture's job names, some optional. */
function jobGroups(repo: RepoSeed, r: () => number): { name: string; steps: StepSeed[] }[] {
  const groups = new Map<string, StepSeed[]>();
  for (const s of repo.steps) groups.set(s.job, [...(groups.get(s.job) ?? []), s]);
  const all = [...groups].map(([name, steps]) => ({ name, steps }));
  // quod: check and web always; the slow integration job sometimes; release only on its own schedule (not simulated).
  if (repo.summary.slug === "quod") return all.filter((g) => g.name === "check" || g.name === "web" || (g.name === "integration" && r() < 0.3));
  return all;
}

function planStep(repo: RepoSeed, s: StepSeed, cycle: number, number: number, r: () => number): Omit<SimStep, "index" | "startAt" | "endAt"> {
  const full = round2(Math.min(s.ranSeconds, 24) * between(r, 0.8, 1.15));
  const changed = (have: number, need: number): StepWhy => ({
    kind: "changed",
    stepKey: s.key,
    firstChange: { path: repo.changedPath, was: `b3:${hex(r, 8)}`, now: `b3:${hex(r, 8)}`, what: "content" },
    otherChanges: Math.floor(r() * 4),
    recordedInputs: s.recordedInputs,
    proof: { have, need },
  });
  const ran = (why: StepWhy) => ({ name: s.command, stepKey: s.key, outcome: "ran" as const, seconds: full, fullSeconds: full, why });

  switch (s.trust.kind) {
    case "proving":
      return ran(changed(1, s.trust.need));
    case "watching":
      return ran({ kind: "watching", stepKey: s.key, couldHaveReplayed: r() < 0.6, recordedInputs: s.recordedInputs });
    case "paused":
      return ran({ kind: "paused", stepKey: s.key, consistent: s.trust.consistent, need: s.trust.need });
    case "replaying": {
      const replays = r() < Math.max(0.55, s.replayRate ?? 0);
      if (replays && r() < 0.08) {
        return ran({ kind: "spot-check", stepKey: s.key, sourceRun: number - 1, inputsMatched: s.recordedInputs, identical: true });
      }
      if (!replays) return ran(changed(1, 2));
      const seconds = round2((s.replaySeconds ?? 0.3) * between(r, 0.8, 1.25));
      return {
        name: s.command,
        stepKey: s.key,
        outcome: "replayed",
        seconds,
        fullSeconds: full,
        why: { kind: "replayed", stepKey: s.key, sourceRun: number - 1 - Math.floor(r() * 3), inputsMatched: s.recordedInputs, proof: { have: 2, need: 2 }, divergences: 0, savedSeconds: full },
      };
    }
  }
}

const cache = new Map<number, SimRun>();

/** The run that cycle `cycle` starts. Pure: depends only on the cycle number. */
export function planCycle(cycle: number): SimRun {
  const hit = cache.get(cycle);
  if (hit) return hit;
  const r = rng(Math.imul(cycle + 1, 2654435761) ^ 0x51ed270b);
  const repo = repoBySlug(PATTERN[((cycle % PATTERN.length) + PATTERN.length) % PATTERN.length])!;
  const number = numberFor(cycle, repo);
  const startedAt = cycleStart(cycle);
  const pr = r() < 0.25;

  const jobs: SimJob[] = jobGroups(repo, r).map((g, j) => {
    const queuedAt = startedAt + 150 * j;
    // Most jobs get a runner in a second or two; some wait for one, so the "queued" state shows.
    const wait = j === 0 ? between(r, 600, 2500) : r() < 0.4 ? between(r, 6000, 16000) : between(r, 800, 3500);
    const jobStart = Math.round(queuedAt + wait);
    const seeds = [
      { name: "Set up job", seconds: round2(between(r, 2.2, 3.4)), why: { kind: "not-memoized", what: "runner setup" } as StepWhy },
      { name: "Checkout", seconds: round2(between(r, 0.8, 1.6)), why: { kind: "not-memoized", what: "checkout" } as StepWhy },
    ];
    const steps: SimStep[] = [];
    let at = jobStart;
    for (const [i, s] of seeds.entries()) {
      const end = Math.round(at + s.seconds * 1000);
      steps.push({ index: i, name: s.name, stepKey: null, outcome: "ran", seconds: s.seconds, fullSeconds: s.seconds, why: s.why, startAt: at, endAt: end });
      at = end + 150;
    }
    for (const s of g.steps) {
      const p = planStep(repo, s, cycle, number, r);
      const end = Math.round(at + p.seconds * 1000);
      steps.push({ ...p, index: steps.length, startAt: at, endAt: end });
      at = end + 150;
    }
    return { id: `${repo.summary.slug}-${number}-${j}`, name: g.name, queuedAt, startedAt: jobStart, finishedAt: steps[steps.length - 1].endAt, steps };
  });

  const run: SimRun = {
    cycle,
    repo,
    number,
    title: TITLES[Math.abs(cycle * 7 + 3) % TITLES.length],
    branch: pr ? "feature/receipts" : repo.defaultBranch,
    sha: hex(r, 40),
    actor: ACTORS[Math.abs(cycle) % ACTORS.length],
    event: pr ? "pull_request" : "push",
    startedAt,
    finishedAt: Math.max(...jobs.map((j) => j.finishedAt)),
    jobs,
  };
  if (cache.size > 2000) cache.clear();
  cache.set(cycle, run);
  return run;
}

// ---------------------------------------------------------------- State of a run at a moment

const iso = (t: number) => new Date(t).toISOString();

function stepAt(s: SimStep, t: number): RunStep {
  if (t >= s.endAt) return { name: s.name, outcome: s.outcome, seconds: s.seconds, why: s.why };
  if (t >= s.startAt) return { name: s.name, outcome: "running", seconds: round2((t - s.startAt) / 1000), startedAt: iso(s.startAt), why: { kind: "running", stepKey: s.stepKey } };
  return { name: s.name, outcome: "queued", seconds: 0, why: { kind: "queued", stepKey: s.stepKey } };
}

export function summaryAt(run: SimRun, t: number): RunSummary {
  const done = run.jobs.flatMap((j) => j.steps).filter((s) => s.endAt <= t);
  const spot = done.filter((s) => s.why.kind === "spot-check");
  const wouldHave = Math.max(...run.jobs.map((j) => (j.startedAt - j.queuedAt) / 1000 + j.steps.reduce((a, s) => a + s.fullSeconds, 0)));
  return {
    repoSlug: run.repo.summary.slug,
    number: run.number,
    title: run.title,
    branch: run.branch,
    sha: run.sha,
    actor: run.actor,
    event: run.event,
    runner: run.repo.runner,
    status: t >= run.finishedAt ? "passed" : "running",
    startedAt: iso(run.startedAt),
    tookSeconds: round2((Math.min(t, run.finishedAt) - run.startedAt) / 1000),
    wouldHaveTakenSeconds: round2(wouldHave),
    stepsReplayed: done.filter((s) => s.outcome === "replayed").length,
    stepsTotal: run.jobs.reduce((a, j) => a + j.steps.length, 0),
    spotChecks: spot.length,
    spotChecksIdentical: spot.length,
  };
}

export function detailAt(run: SimRun, t: number): RunDetail {
  return {
    ...summaryAt(run, t),
    githubUrl: `https://github.com/${run.repo.summary.fullName}/actions`,
    jobs: run.jobs.map((j) => ({ name: `${j.name} / ubuntu-24.04`, steps: j.steps.map((s) => stepAt(s, t)) })),
  };
}

export function jobAt(run: SimRun, job: SimJob, t: number): LiveJob {
  const current = job.steps.find((s) => s.startAt <= t && t < s.endAt) ?? [...job.steps].reverse().find((s) => s.startAt <= t);
  const status = t < job.startedAt ? "queued" : t < job.finishedAt ? "in_progress" : "completed";
  return {
    id: job.id,
    repoSlug: run.repo.summary.slug,
    repoName: run.repo.summary.fullName,
    runNumber: run.number,
    workflow: "CI",
    job: job.name,
    branch: run.branch,
    sha: run.sha,
    actor: run.actor,
    runner: run.repo.summary.runner === "self-hosted" ? "[self-hosted, quod]" : "ubuntu-latest",
    githubUrl: `https://github.com/${run.repo.summary.fullName}/actions`,
    status,
    ...(status === "completed" ? { conclusion: "success" as const, finishedAt: iso(job.finishedAt) } : {}),
    queuedAt: iso(job.queuedAt),
    ...(status !== "queued" ? { startedAt: iso(job.startedAt) } : {}),
    ...(status === "in_progress" && current ? { currentStep: { index: current.index, name: current.name } } : {}),
    stepsTotal: job.steps.length,
  };
}

/** Runs that have started by `t`, newest first, back to `windowMs` ago. */
export function runsStartedBetween(from: number, t: number, repo?: string): SimRun[] {
  const out: SimRun[] = [];
  for (let c = cycleAt(t); c >= 0 && cycleStart(c) > from; c--) {
    const run = planCycle(c);
    if (!repo || run.repo.summary.slug === repo) out.push(run);
  }
  return out;
}

/** The repo's most recent simulated run that has started by `t`. */
export function latestRun(repo: string, t: number): SimRun | null {
  return runsStartedBetween(t - PATTERN.length * PERIOD_MS * 2, t, repo)[0] ?? null;
}

/** A simulated run by number, as it stands at `t`; null if it has not started yet. */
export function runByNumber(repo: string, number: number, t: number): SimRun | null {
  const c = cycleFor(repo, number);
  if (c === null || cycleStart(c) > t) return null;
  return planCycle(c);
}

/** Jobs queued or in progress at `t`. */
export function jobsAt(t: number, repo?: string): LiveJob[] {
  return runsStartedBetween(t - MAX_RUN_MS, t, repo)
    .flatMap((run) => run.jobs.filter((j) => j.queuedAt <= t && t < j.finishedAt).map((j) => jobAt(run, j, t)))
    .sort((a, b) => a.queuedAt.localeCompare(b.queuedAt));
}

// ---------------------------------------------------------------- Events

/** Event ids sort by time: "<ms>-<cycle>-<seq>". */
export function eventId(at: number, cycle: number, seq: number): string {
  return `${at}-${cycle}-${seq}`;
}

export function parseEventId(id: string | null | undefined): [number, number, number] | null {
  const m = /^(\d{1,15})-(\d{1,9})-(\d{1,4})$/.exec(id ?? "");
  return m ? [Number(m[1]), Number(m[2]), Number(m[3])] : null;
}

const after = (a: [number, number, number], b: [number, number, number]) => a[0] - b[0] || a[1] - b[1] || a[2] - b[2];

/** All events of one run, in order. */
export function runEvents(run: SimRun): LiveEvent[] {
  const raw: { at: number; make: (id: string) => LiveEvent }[] = [];
  const at = (t: number) => iso(t);
  const slug = run.repo.summary.slug;
  raw.push({ at: run.startedAt, make: (id) => ({ type: "run.started", id, at: at(run.startedAt), run: summaryAt(run, run.startedAt) }) });
  for (const job of run.jobs) {
    raw.push({ at: job.queuedAt, make: (id) => ({ type: "job.queued", id, at: at(job.queuedAt), job: jobAt(run, job, job.queuedAt) }) });
    raw.push({ at: job.startedAt, make: (id) => ({ type: "job.started", id, at: at(job.startedAt), job: jobAt(run, job, job.startedAt) }) });
    for (const s of job.steps) {
      raw.push({ at: s.startAt, make: (id) => ({ type: "step.started", id, at: at(s.startAt), repoSlug: slug, run: run.number, jobId: job.id, index: s.index, name: s.name, stepKey: s.stepKey }) });
      raw.push({ at: s.endAt, make: (id) => ({ type: "step.finished", id, at: at(s.endAt), repoSlug: slug, run: run.number, jobId: job.id, index: s.index, stepKey: s.stepKey, step: stepAt(s, s.endAt) }) });
      if (s.stepKey) {
        const delta: StatsDelta =
          s.outcome === "replayed"
            ? { replayedMinutes: s.fullSeconds / 60, ranMinutes: 0, stepsReplayed: 1, stepsTotal: 1, spotChecks: 0, spotChecksIdentical: 0 }
            : { replayedMinutes: 0, ranMinutes: s.seconds / 60, stepsReplayed: 0, stepsTotal: 1, spotChecks: s.why.kind === "spot-check" ? 1 : 0, spotChecksIdentical: s.why.kind === "spot-check" ? 1 : 0 };
        raw.push({ at: s.endAt, make: (id) => ({ type: "stats.updated", id, at: at(s.endAt), repoSlug: slug, day: at(s.endAt).slice(0, 10), delta }) });
      }
      if (s.why.kind === "spot-check" && s.stepKey) {
        const key = s.stepKey;
        const ev: TrustEvent = { kind: "spot-check-passed", at: at(s.endAt), run: run.number };
        raw.push({ at: s.endAt, make: (id) => ({ type: "trust.changed", id, at: at(s.endAt), repoSlug: slug, key, trust: { kind: "replaying" }, event: ev }) });
      }
    }
    raw.push({ at: job.finishedAt, make: (id) => ({ type: "job.finished", id, at: at(job.finishedAt), job: jobAt(run, job, job.finishedAt) }) });
  }
  raw.push({ at: run.finishedAt, make: (id) => ({ type: "run.finished", id, at: at(run.finishedAt), run: summaryAt(run, run.finishedAt) }) });
  // Stable order: by time, then by creation order. The sequence number is the position after sorting.
  return raw
    .map((e, i) => ({ ...e, i }))
    .sort((a, b) => a.at - b.at || a.i - b.i)
    .map((e, seq) => e.make(eventId(e.at, run.cycle, seq)));
}

/** How far back a resume reaches. Older clients refetch the page instead. */
export const RESUME_WINDOW_MS = 5 * 60_000;

/** Events with time in (after `lastEventId` or `from`, `to`], in order. */
export function eventsBetween(from: number, to: number, opts: { lastEventId?: string | null; scope?: LiveScope } = {}): LiveEvent[] {
  const last = parseEventId(opts.lastEventId);
  const floor = to - RESUME_WINDOW_MS;
  const start = Math.max(last ? last[0] : from, floor);
  const out: LiveEvent[] = [];
  for (let c = Math.max(0, cycleAt(start - MAX_RUN_MS)); cycleStart(c) <= to; c++) {
    for (const e of runEvents(planCycle(c))) {
      const t = Date.parse(e.at);
      if (t > to) continue;
      const key = parseEventId(e.id)!;
      if (t <= floor || (last ? after(key, last) <= 0 : t <= from)) continue;
      if (opts.scope && !inScope(e, opts.scope)) continue;
      out.push(e);
    }
  }
  return out.sort((a, b) => after(parseEventId(a.id)!, parseEventId(b.id)!));
}

/** Spot checks the simulator played for a step key in the last `windowMs`, newest first (step trust history). */
export function trustEventsFor(repo: string, key: string, t: number, windowMs = 30 * 60_000): TrustEvent[] {
  return eventsBetween(t - windowMs, t, { scope: { kind: "step", repo, key } })
    .flatMap((e) => (e.type === "trust.changed" ? [e.event] : []))
    .reverse();
}
