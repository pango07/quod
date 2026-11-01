// The CloudApi: everything the dashboard reads or changes, shaped by the
// screens in the approved design (DECISIONS.md, 2026-11-01). Pages read only
// through this interface. Two adapters implement it: fixture.ts (sample data
// from the prototypes, used now) and http.ts (quod-server, protocol
// "quod.v1", not built yet). docs/api.md is the prose version.
//
// Conventions: times are ISO 8601 strings in UTC, durations are seconds,
// saved or spent time is whole minutes, shares are 0..1. Every value is plain
// JSON so the HTTP adapter can return the server's body as is.

export type Range = "7d" | "30d" | "90d";
export const RANGES: readonly Range[] = ["7d", "30d", "90d"];

/** A team is one WorkOS Organization. Quod owns the slug; WorkOS owns members and SSO. */
export interface Team {
  slug: string;
  name: string;
  plan: "Hobby" | "Pro" | "Enterprise";
  /** The WorkOS Organization id this team maps to. */
  orgId: string;
}

export interface OrgRef {
  orgId: string;
  name: string;
}

export type RunnerKind = "self-hosted" | "github-hosted";

// ---------------------------------------------------------------- Overview

export interface TeamTotals {
  range: Range;
  /** CI minutes not waited for, because steps were replayed. */
  savedMinutes: number;
  stepsReplayed: number;
  stepsTotal: number;
  /** savedMinutes priced at usdPerMinute. */
  costSavedUsd: number;
  usdPerMinute: number;
  /** Replays a spot check found to differ from a real run. The trust claim; should be 0. */
  wrongReplays: number;
  /** Replays re-run at random and compared (about 5%). */
  spotChecks: number;
  spotChecksIdentical: number;
}

export interface DayMinutes {
  /** YYYY-MM-DD, UTC. */
  day: string;
  replayedMinutes: number;
  ranMinutes: number;
}

export interface RepoSummary {
  /** URL segment, unique within the team: /[team]/[repo]. */
  slug: string;
  /** owner/name on GitHub. */
  fullName: string;
  language: string;
  runner: RunnerKind;
  /** Share of steps replayed in the range. */
  replayRate: number;
  savedMinutes: number;
  lastRunAt: string | null;
  lastRunStatus: RunStatus | null;
  pausedSteps: number;
}

export interface PausedStepRef {
  repoSlug: string;
  repoName: string;
  key: string;
  job: string;
  command: string;
}

export interface TrustTotals {
  proven: number;
  proving: number;
  watching: number;
  spotChecksPassed: number;
  paused: PausedStepRef[];
}

export interface Runner {
  name: string;
  kind: RunnerKind;
  status: "idle" | "running" | "offline";
}

export interface Overview {
  /** The server's clock; relative times ("4m ago") are measured from it. */
  asOf: string;
  totals: TeamTotals;
  days: DayMinutes[];
  repos: RepoSummary[];
  trust: TrustTotals;
  runners: Runner[];
}

// ---------------------------------------------------------------- Runs

export type RunStatus = "passed" | "failed" | "running";

export interface RunSummary {
  repoSlug: string;
  /** The GitHub run number, unique within the repo: /[team]/[repo]/runs/[number]. */
  number: number;
  title: string;
  branch: string;
  sha: string;
  actor: string;
  event: "push" | "pull_request" | "schedule" | "workflow_dispatch";
  runner: string;
  status: RunStatus;
  startedAt: string;
  tookSeconds: number;
  /** What the run would have taken with every step executed (recorded times). */
  wouldHaveTakenSeconds: number;
  stepsReplayed: number;
  stepsTotal: number;
  spotChecks: number;
  spotChecksIdentical: number;
}

export interface Proof {
  have: number;
  need: number;
}

/** Why a step in a run replayed or ran: the receipt behind the Replayed/Ran chip. */
export type StepWhy =
  | {
      kind: "replayed";
      stepKey: string;
      /** The run whose result was given back. */
      sourceRun: number;
      inputsMatched: number;
      proof: Proof;
      divergences: number;
      savedSeconds: number;
    }
  | {
      kind: "spot-check";
      stepKey: string;
      sourceRun: number;
      inputsMatched: number;
      identical: boolean;
    }
  | {
      kind: "changed";
      stepKey: string;
      /** The first input that differs from the proven run, in read order. */
      firstChange: { path: string; was: string; now: string; what: "content" | "missing" | "created" | "listing" };
      otherChanges: number;
      recordedInputs: number;
      /** Where this new input set now stands. */
      proof: Proof;
    }
  | { kind: "first-run"; stepKey: string; recordedInputs: number; proof: Proof }
  | { kind: "paused"; stepKey: string; consistent: number; need: number }
  /** Observe mode: it ran and was recorded. `couldHaveReplayed`: nothing it reads changed. */
  | { kind: "watching"; stepKey: string; couldHaveReplayed: boolean; recordedInputs: number }
  | { kind: "network" }
  | { kind: "not-memoized"; what: "runner setup" | "checkout" | "cleanup" };

export interface RunStep {
  name: string;
  outcome: "replayed" | "ran";
  seconds: number;
  why: StepWhy;
}

export interface RunJob {
  /** "check / ubuntu-24.04" */
  name: string;
  steps: RunStep[];
}

export interface RunDetail extends RunSummary {
  githubUrl: string;
  jobs: RunJob[];
}

// ---------------------------------------------------------------- Steps and trust

export type TrustState =
  /** Proven; replays when nothing it reads changed. */
  | { kind: "replaying" }
  /** Same inputs gave the same result `have` of `need` times so far. */
  | { kind: "proving"; have: number; need: number }
  /** Observe mode: runs every time and is recorded; nothing is skipped yet. */
  | { kind: "watching"; wouldHaveReplayed: number }
  /** A spot check differed. Runs every time until `need` consistent runs in a row. */
  | { kind: "paused"; consistent: number; need: number };

export interface StepSummary {
  repoSlug: string;
  /** The step key: /[team]/[repo]/steps/[key]. */
  key: string;
  job: string;
  command: string;
  trust: TrustState;
  /** Share of runs replayed, null while it cannot replay. */
  replayRate: number | null;
  /** Typical time when it runs. */
  ranSeconds: number;
  /** Typical time when it replays, null if it never has. */
  replaySeconds: number | null;
  savedMinutes: number;
}

export interface RunTimePoint {
  number: number;
  withQuodSeconds: number;
  /** Recorded time of every step executed. */
  withoutSeconds: number;
}

export interface RepoDetail {
  asOf: string;
  summary: RepoSummary;
  defaultBranch: string;
  latestRun: RunDetail | null;
  /** Oldest first. */
  runTimes: RunTimePoint[];
  steps: StepSummary[];
}

export type TrustEvent =
  | { kind: "first-recorded"; at: string; run: number; seconds: number }
  | { kind: "proven"; at: string; runs: [number, number] }
  | { kind: "replayed"; at: string; times: number; savedMinutes: number }
  | { kind: "spot-check-passed"; at: string; run: number }
  | { kind: "paused"; at: string; run: number }
  | { kind: "consistent"; at: string; run: number; sameAs: number }
  | { kind: "proofs-changed"; at: string; from: number; to: number; by: string };

export interface Divergence {
  recordedRun: number;
  recheckRun: number;
  inputs: number;
  /** A unified-diff-like excerpt of the first difference. */
  stream: "stdout" | "stderr" | "outputs";
  line: number;
  recorded: string[];
  recheck: string[];
  exitCode: { recorded: number; recheck: number };
}

export interface InputCounts {
  filesRead: number;
  foldersListed: number;
  pathsNotFound: number;
  networkAccess: number;
}

export interface StepTrust extends StepSummary {
  asOf: string;
  /** Consistent runs needed before it replays again (or for the first time). */
  proofsNeeded: number;
  divergence: Divergence | null;
  /** Newest first. */
  history: TrustEvent[];
  inputs: InputCounts;
}

// ---------------------------------------------------------------- Onboarding

export type Store = "cloud" | "self";
export type Host = "github" | "own";

export interface ConnectSetup {
  /** The GitHub Action for GitHub-hosted runners. */
  setupAction: string;
  /** runs-on labels for runners with Quod installed. */
  runnerLabels: string[];
  installCommand: string;
  /** Example quod-server URL for the self-hosted choice. */
  selfHostedExample: string;
  /** A repo to open when done; null for a team with none yet. */
  firstRepo: { slug: string; fullName: string } | null;
  /** The team's saved choice, if any. */
  saved: { store: Store; host: Host } | null;
}

// ---------------------------------------------------------------- Team health (sidebar)

/** All-time trust numbers, cheap to read on every page. */
export interface TeamHealth {
  wrongReplays: number;
  spotChecks: number;
  pausedSteps: number;
  repos: number;
}

// ---------------------------------------------------------------- Command menu

export interface SearchIndex {
  repos: { slug: string; fullName: string }[];
  runs: { repoSlug: string; number: number; title: string; status: RunStatus }[];
  steps: { repoSlug: string; key: string; command: string; trust: TrustState["kind"] }[];
}

// ---------------------------------------------------------------- The interface

export interface CloudApi {
  /** The teams for the viewer's WorkOS Organizations. Unknown orgs come back as new, empty teams. */
  resolveTeams(orgs: OrgRef[]): Promise<Team[]>;

  getTeamHealth(team: string): Promise<TeamHealth>;
  getOverview(team: string, range: Range): Promise<Overview>;
  listRepos(team: string): Promise<RepoSummary[]>;
  getRepo(team: string, repo: string): Promise<RepoDetail | null>;

  /** Newest first. Without `repo`, across the team. */
  listRuns(team: string, opts?: { repo?: string; limit?: number }): Promise<{ asOf: string; runs: RunSummary[] }>;
  getRun(team: string, repo: string, run: number): Promise<RunDetail | null>;

  /** Without `repo`, across the team. */
  listSteps(team: string, opts?: { repo?: string }): Promise<StepSummary[]>;
  getStepTrust(team: string, repo: string, key: string): Promise<StepTrust | null>;
  setProofsNeeded(team: string, repo: string, key: string, need: number): Promise<StepTrust>;

  /** Ask GitHub for a fresh run with every step executed. */
  requestRerunWithoutQuod(team: string, repo: string, run: number): Promise<{ accepted: boolean; message: string }>;

  listRunners(team: string): Promise<Runner[]>;
  getConnectSetup(team: string): Promise<ConnectSetup>;
  saveConnectChoice(team: string, choice: { store: Store; host: Host }): Promise<void>;

  getSearchIndex(team: string): Promise<SearchIndex>;
}

/** Thrown by an adapter when the backend refuses the viewer: 401 (sign in again) or 403 (not theirs). */
export class ApiForbiddenError extends Error {
  constructor(
    message = "Not allowed",
    readonly status: 401 | 403 = 403,
  ) {
    super(message);
    this.name = "ApiForbiddenError";
  }
}

/** Thrown before a request when a path segment is not a valid id; shown as not found. */
export class ApiInvalidRequestError extends Error {
  constructor(message = "Invalid request") {
    super(message);
    this.name = "ApiInvalidRequestError";
  }
}
