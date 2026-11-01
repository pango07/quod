// The fixture adapter: CloudApi over the prototypes' sample data, in memory.
// Changes (proofs needed, onboarding choice) last until the process restarts.
// Set QUOD_FIXTURE_DELAY_MS to slow every call down and watch the skeletons.

import {
  asOf,
  CONNECT,
  daysFor,
  EMPTY_TOTALS,
  REPOS,
  RUNNERS,
  RUNS_PER_REPO,
  runDetail,
  runTimes,
  stepSummary,
  TEAMS,
  totalsFor,
  TRUST_COUNTS,
  trustSeed,
} from "./fixture-data";
import { cursorAt, deltaSinceMidnight, detailAt, eventsBetween, jobsAt, latestRun, runByNumber, runsStartedBetween, summaryAt, trustEventsFor } from "@/lib/live/sim";
import type {
  Activity,
  LiveEvent,
  LiveScope,
  CloudApi,
  ConnectSetup,
  Host,
  OrgRef,
  Overview,
  Range,
  RepoDetail,
  RepoSummary,
  RunDetail,
  RunSummary,
  SearchIndex,
  StepSummary,
  StepTrust,
  Store,
  Team,
  TeamHealth,
  TrustState,
} from "./types";

type RepoSeed = (typeof REPOS)[number];
type StepSeed = RepoSeed["steps"][number];

/** The one team with sample data. Other teams are real but empty until a repo connects. */
const SAMPLE_TEAM = "quodci";

/** The sample team plays live CI runs (lib/live/sim.ts) unless QUOD_SIMULATE=0 turns it off. */
const simulating = (team: string) => team === SAMPLE_TEAM && process.env.QUOD_SIMULATE !== "0";

const sleep = (ms: number, signal: AbortSignal) =>
  new Promise<void>((resolve) => {
    const t = setTimeout(resolve, ms);
    signal.addEventListener("abort", () => (clearTimeout(t), resolve()), { once: true });
  });

const slugify = (s: string) =>
  s
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 40) || "team";

export interface FixtureState {
  proofs: Map<string, number>;
  choices: Map<string, { store: Store; host: Host }>;
}

export const newFixtureState = (): FixtureState => ({ proofs: new Map(), choices: new Map() });

export class FixtureCloudApi implements CloudApi {
  private readonly proofs: Map<string, number>;
  private readonly choices: Map<string, { store: Store; host: Host }>;

  /** `state` holds what the UI changed; pass the same object to keep it across instances. */
  constructor(
    state: FixtureState = newFixtureState(),
    private readonly delayMs = Number(process.env.QUOD_FIXTURE_DELAY_MS ?? 0),
  ) {
    this.proofs = state.proofs;
    this.choices = state.choices;
  }

  private async tick() {
    if (this.delayMs > 0) await new Promise((r) => setTimeout(r, this.delayMs));
  }

  private repos(team: string): RepoSeed[] {
    return team === SAMPLE_TEAM ? REPOS : [];
  }

  private repo(team: string, slug: string): RepoSeed | undefined {
    return this.repos(team).find((r) => r.summary.slug === slug);
  }

  /** Proofs needed for a step, keyed by team/repo/key (sample repos all belong to SAMPLE_TEAM). */
  private needIn = (repo: RepoSeed) => (s: StepSeed): number => {
    const base = s.trust.kind === "paused" || s.trust.kind === "proving" ? s.trust.need : 2;
    return this.proofs.get(`${SAMPLE_TEAM}/${repo.summary.slug}/${s.key}`) ?? base;
  };

  private trustOf(repo: RepoSeed, s: StepSeed): TrustState {
    const need = this.needIn(repo)(s);
    if (s.trust.kind === "paused") return { ...s.trust, need, consistent: Math.min(s.trust.consistent, need) };
    if (s.trust.kind === "proving") return { ...s.trust, need, have: Math.min(s.trust.have, need) };
    return s.trust;
  }

  /** The repo's summary, with its last run taken from the simulator when it is live. */
  private liveSummary(team: string, repo: RepoSeed, now = Date.now()): RepoSummary {
    const latest = simulating(team) ? latestRun(repo.summary.slug, now) : null;
    if (!latest) return { ...repo.summary };
    return { ...repo.summary, lastRunAt: new Date(latest.startedAt).toISOString(), lastRunStatus: now < latest.finishedAt ? "running" : "passed" };
  }

  private summaries(repo: RepoSeed): StepSummary[] {
    return repo.steps.filter((s) => !s.network).map((s) => stepSummary(repo, s, this.trustOf(repo, s)));
  }

  private runSummaries(repo: RepoSeed): RunSummary[] {
    const out: RunSummary[] = [];
    for (let n = repo.firstRun + RUNS_PER_REPO - 1; n >= repo.firstRun; n--) {
      const d = runDetail(repo, n, this.needIn(repo));
      if (d) {
        const { jobs: _jobs, githubUrl: _url, ...summary } = d;
        void _jobs;
        void _url;
        out.push(summary);
      }
    }
    return out;
  }

  async resolveTeams(orgs: OrgRef[]): Promise<Team[]> {
    await this.tick();
    const taken = new Set<string>();
    return orgs.map((org) => {
      const known = TEAMS.find((t) => t.orgId === org.orgId);
      if (known) {
        taken.add(known.slug);
        return known;
      }
      // Sample data is opt-in (QUOD_API=fixture), and teams are only created in
      // the WorkOS dashboard, so a real Organization named like a sample team
      // (e.g. "quodci") is that team: it shows the sample data under its own org id.
      const sample = TEAMS.find((t) => t.slug === slugify(org.name) && !taken.has(t.slug));
      if (sample) {
        taken.add(sample.slug);
        return { ...sample, name: org.name, orgId: org.orgId };
      }
      let slug = slugify(org.name);
      for (let i = 2; taken.has(slug) || TEAMS.some((t) => t.slug === slug); i++) slug = `${slugify(org.name)}-${i}`;
      taken.add(slug);
      return { slug, name: org.name, plan: "Hobby", orgId: org.orgId } satisfies Team;
    });
  }

  async getTeamHealth(team: string): Promise<TeamHealth> {
    await this.tick();
    const repos = this.repos(team);
    const all = totalsFor("90d");
    return {
      wrongReplays: repos.length ? all.wrongReplays : 0,
      spotChecks: repos.length ? all.spotChecks : 0,
      pausedSteps: repos.reduce((a, r) => a + r.steps.filter((s) => s.trust.kind === "paused").length, 0),
      repos: repos.length,
    };
  }

  async getOverview(team: string, range: Range): Promise<Overview> {
    await this.tick();
    const repos = this.repos(team);
    if (repos.length === 0) {
      return {
        asOf: asOf(),
        totals: EMPTY_TOTALS(range),
        days: [],
        repos: [],
        trust: { proven: 0, proving: 0, watching: 0, spotChecksPassed: 0, paused: [] },
        runners: [],
      };
    }
    const base7 = totalsFor(range);
    const days = daysFor(range);
    // Fold in what the simulator added today, so a reload shows what the open page had reached.
    // Whole numbers, like the rest of the sample data (the per-repo split below rounds too).
    const raw = simulating(team) ? deltaSinceMidnight(Date.now()) : null;
    const sim = raw
      ? {
          replayedMinutes: Math.round(raw.replayedMinutes),
          ranMinutes: Math.round(raw.ranMinutes),
          stepsReplayed: Math.round(raw.stepsReplayed),
          stepsTotal: Math.round(raw.stepsTotal),
          spotChecks: Math.round(raw.spotChecks),
          spotChecksIdentical: Math.round(raw.spotChecksIdentical),
        }
      : null;
    const totals = sim
      ? {
          ...base7,
          savedMinutes: base7.savedMinutes + sim.replayedMinutes,
          costSavedUsd: (base7.savedMinutes + sim.replayedMinutes) * base7.usdPerMinute,
          stepsReplayed: base7.stepsReplayed + sim.stepsReplayed,
          stepsTotal: base7.stepsTotal + sim.stepsTotal,
          spotChecks: base7.spotChecks + sim.spotChecks,
          spotChecksIdentical: base7.spotChecksIdentical + sim.spotChecksIdentical,
        }
      : base7;
    if (sim && days.length) {
      const last = days[days.length - 1];
      days[days.length - 1] = { ...last, replayedMinutes: last.replayedMinutes + sim.replayedMinutes, ranMinutes: last.ranMinutes + sim.ranMinutes };
    }
    // Repo savings are given for 7 days; scale them so they add up to the range's total.
    const base = repos.reduce((a, r) => a + r.summary.savedMinutes, 0);
    let left = totals.savedMinutes;
    const scaled: RepoSummary[] = repos.map((r, i) => {
      const saved = i === repos.length - 1 ? left : Math.round((r.summary.savedMinutes / base) * totals.savedMinutes);
      left -= saved;
      return { ...this.liveSummary(team, r), savedMinutes: saved };
    });
    const paused = repos.flatMap((r) =>
      r.steps
        .filter((s) => s.trust.kind === "paused")
        .map((s) => ({ repoSlug: r.summary.slug, repoName: r.summary.fullName, key: s.key, job: s.job, command: s.command })),
    );
    return {
      asOf: asOf(),
      totals,
      days,
      repos: scaled,
      trust: { ...TRUST_COUNTS, spotChecksPassed: totals.spotChecksIdentical, paused },
      runners: RUNNERS.map((r) => ({ ...r })),
    };
  }

  async listRepos(team: string): Promise<RepoSummary[]> {
    await this.tick();
    return this.repos(team).map((r) => this.liveSummary(team, r));
  }

  async getRepo(team: string, slug: string): Promise<RepoDetail | null> {
    await this.tick();
    const repo = this.repo(team, slug);
    if (!repo) return null;
    const last = repo.firstRun + RUNS_PER_REPO - 1;
    const now = Date.now();
    const live = simulating(team) ? latestRun(slug, now) : null;
    return {
      asOf: asOf(),
      summary: this.liveSummary(team, repo, now),
      defaultBranch: repo.defaultBranch,
      latestRun: live ? detailAt(live, now) : runDetail(repo, last, this.needIn(repo)),
      runTimes: runTimes(repo),
      steps: this.summaries(repo),
    };
  }

  async listRuns(team: string, opts: { repo?: string; limit?: number } = {}): Promise<{ asOf: string; runs: RunSummary[] }> {
    await this.tick();
    const repos = opts.repo ? this.repos(team).filter((r) => r.summary.slug === opts.repo) : this.repos(team);
    const now = Date.now();
    const live = simulating(team) ? runsStartedBetween(now - 10 * 60_000, now, opts.repo).map((r) => summaryAt(r, now)) : [];
    const runs = [...live, ...repos.flatMap((r) => this.runSummaries(r))].sort((a, b) => b.startedAt.localeCompare(a.startedAt));
    return { asOf: asOf(), runs: opts.limit ? runs.slice(0, opts.limit) : runs };
  }

  async getRun(team: string, slug: string, run: number): Promise<RunDetail | null> {
    await this.tick();
    const repo = this.repo(team, slug);
    if (!repo) return null;
    const now = Date.now();
    const live = simulating(team) ? runByNumber(slug, run, now) : null;
    return live ? detailAt(live, now) : runDetail(repo, run, this.needIn(repo));
  }

  async listSteps(team: string, opts: { repo?: string } = {}): Promise<StepSummary[]> {
    await this.tick();
    const repos = opts.repo ? this.repos(team).filter((r) => r.summary.slug === opts.repo) : this.repos(team);
    return repos.flatMap((r) => this.summaries(r));
  }

  async getStepTrust(team: string, slug: string, key: string): Promise<StepTrust | null> {
    await this.tick();
    const repo = this.repo(team, slug);
    const step = repo?.steps.find((s) => s.key === key && !s.network);
    if (!repo || !step) return null;
    const seed = trustSeed(repo, step);
    const live = simulating(team) ? trustEventsFor(slug, key, Date.now()) : [];
    return { ...stepSummary(repo, step, this.trustOf(repo, step)), asOf: asOf(), proofsNeeded: this.needIn(repo)(step), ...seed, history: [...live, ...seed.history] };
  }

  async setProofsNeeded(team: string, slug: string, key: string, need: number): Promise<StepTrust> {
    if (!Number.isInteger(need) || need < 2 || need > 10) throw new Error("Proofs needed must be a whole number from 2 to 10.");
    const repo = this.repo(team, slug);
    if (!repo?.steps.some((s) => s.key === key)) throw new Error("No such step.");
    this.proofs.set(`${team}/${slug}/${key}`, need);
    const trust = await this.getStepTrust(team, slug, key);
    if (!trust) throw new Error("No such step.");
    return trust;
  }

  async requestRerunWithoutQuod(team: string, slug: string, run: number): Promise<{ accepted: boolean; message: string }> {
    await this.tick();
    if (!this.repo(team, slug)) return { accepted: false, message: "No such repository." };
    return {
      accepted: true,
      message: `Sample data: nothing was sent to GitHub. With quod-server connected, run #${run} starts again with every step executed.`,
    };
  }

  async listRunners(team: string) {
    await this.tick();
    return team === SAMPLE_TEAM ? RUNNERS.map((r) => ({ ...r })) : [];
  }

  async getConnectSetup(team: string): Promise<ConnectSetup> {
    await this.tick();
    const first = this.repos(team)[0];
    return {
      ...CONNECT,
      runnerLabels: [...CONNECT.runnerLabels],
      firstRepo: first ? { slug: first.summary.slug, fullName: first.summary.fullName } : null,
      saved: this.choices.get(team) ?? null,
    };
  }

  async saveConnectChoice(team: string, choice: { store: Store; host: Host }): Promise<void> {
    await this.tick();
    this.choices.set(team, choice);
  }

  async getActivity(team: string, opts: { repo?: string } = {}): Promise<Activity> {
    await this.tick();
    const now = Date.now();
    const live = simulating(team);
    return { asOf: new Date(now).toISOString(), jobs: live ? jobsAt(now, opts.repo) : [], simulated: live, cursor: live ? cursorAt(now) : null };
  }

  /** Polls the simulator's clock twice a second; a resume replays what was missed (up to 5 minutes). */
  async *subscribe(team: string, scope: LiveScope, opts: { lastEventId?: string | null; signal: AbortSignal }): AsyncIterable<LiveEvent> {
    if (!simulating(team)) {
      // Nothing happens in a team without sample data: hold the stream open quietly.
      while (!opts.signal.aborted) await sleep(5000, opts.signal);
      return;
    }
    let cursor = opts.lastEventId ?? null;
    let from = Date.now();
    while (!opts.signal.aborted) {
      const now = Date.now();
      for (const e of eventsBetween(from, now, { lastEventId: cursor, scope })) {
        if (opts.signal.aborted) return;
        cursor = e.id;
        yield e;
      }
      from = now;
      await sleep(500, opts.signal);
    }
  }

  async getSearchIndex(team: string): Promise<SearchIndex> {
    await this.tick();
    const repos = this.repos(team);
    return {
      repos: repos.map((r) => ({ slug: r.summary.slug, fullName: r.summary.fullName })),
      runs: repos.flatMap((r) =>
        this.runSummaries(r)
          .slice(0, 6)
          .map((x) => ({ repoSlug: x.repoSlug, number: x.number, title: x.title, status: x.status })),
      ),
      steps: repos.flatMap((r) => this.summaries(r).map((s) => ({ repoSlug: s.repoSlug, key: s.key, command: s.command, trust: s.trust.kind }))),
    };
  }
}
