// CloudApi composite: CI job activity from the GitHub App, everything else
// (Quod's runs, steps, trust, stats) from the inner adapter (fixture or HTTP).
//
// For a team with a linked GitHub installation, "Now running" and the job.*
// events come from Postgres (lib/github/live.ts), whatever QUOD_API is; the
// repo list also shows the installation's repos with their latest workflow
// run, before Quod has data for them. The sample team keeps the simulator.
// Without a database, or when it does not answer, the inner adapter's
// answers stand and Activity.github says why (pages render a notice).

import type { Db } from "@/lib/db/types";
import { teamIdOf, type TeamId } from "@/lib/github/team";
import type { LiveJobEvent } from "@/lib/live-events";
import { joinCursor, splitCursor } from "@/lib/live/cursor";
import { mergeAsync } from "@/lib/live/merge";
import { inScope } from "@/lib/live/scope";
import { isSlug } from "@/lib/params";
import { TEAM_PAGES } from "@/lib/routes";
import type { Activity, CloudApi, GitHubLink, LiveEvent, LiveJob, LiveScope, Overview, RepoDetail, RepoSummary } from "./types";

/** What the composite needs from the GitHub side (lib/github/live.ts and friends); injectable for tests. */
export interface GitHubSource {
  /** Is there a database to read? (DATABASE_URL set.) */
  configured(): boolean;
  /** Is the GitHub App set up on this deploy (so admins can install it)? */
  appReady(): boolean;
  db(): Promise<Db>;
}

export interface CompositeOptions {
  github: GitHubSource;
  /** The team's WorkOS organization id (its TeamId), from the membership-checked team context; null if not visible. */
  orgOf: (team: string) => Promise<string | null>;
  /** How far back a new stream replays GitHub job changes, to cover commits that raced its snapshot. */
  resyncWindowMs?: number;
  /** A fixed poll interval for the job feed, no idle back-off (tests). Default: the poller's own (2 s, backing off to 10 s). */
  pollIntervalMs?: number;
}

type GitHubState =
  | { link: GitHubLink; teamId: null }
  | { link: Extract<GitHubLink, { state: "linked" }> | Extract<GitHubLink, { state: "unlinked" }>; teamId: TeamId; db: Db; repos: { id: number; fullName: string; slug: string }[] };

const RESYNC_MS = 30_000;
const live = () => import("@/lib/github/live");

/** GitHub numbers steps from 1; LiveJob.currentStep.index counts from 0 (like Quod's step events). */
function fromGitHub<J extends LiveJob>(job: J): J {
  return job.currentStep ? { ...job, currentStep: { ...job.currentStep, index: Math.max(0, job.currentStep.index - 1) } } : job;
}

const ghVersion = (id: string) => (/^gh-(\d{1,15})$/.exec(id) ? Number(id.slice(3)) : 0);

export class CompositeCloudApi implements CloudApi {
  constructor(
    private readonly inner: CloudApi,
    private readonly opts: CompositeOptions,
  ) {}

  /** The team's GitHub status, and its repos when linked. Never throws. */
  private async github(team: string): Promise<GitHubState> {
    const gh = this.opts.github;
    if (!gh.configured()) return { link: { state: "not-configured" }, teamId: null };
    try {
      const orgId = await this.opts.orgOf(team);
      if (!orgId) return { link: { state: "unlinked", canInstall: false }, teamId: null };
      const teamId = teamIdOf({ team: { orgId } });
      const db = await gh.db();
      const { teamInstallations } = await live();
      const installs = await teamInstallations(teamId, db);
      const active = installs.filter((i) => !i.suspended);
      if (!active.length) return { link: { state: "unlinked", canInstall: gh.appReady() }, teamId, db, repos: [] };
      // Repos show under their GitHub name; a name that is not a usable slug, or that a team page owns, is left out.
      const seen = new Set<string>();
      const repos = active
        .flatMap((i) => i.repos)
        .map((r) => ({ id: r.id, fullName: r.fullName, slug: r.fullName.split("/")[1] ?? "" }))
        .filter((r) => isSlug(r.slug) && !TEAM_PAGES.includes(r.slug) && !seen.has(r.slug) && (seen.add(r.slug), true));
      return { link: { state: "linked", repos: repos.length }, teamId, db, repos };
    } catch (e) {
      console.warn("[quod-cloud] github source unavailable", { error: e instanceof Error ? e.name : "unknown" });
      return { link: { state: "unavailable" }, teamId: null };
    }
  }

  /** Repos from GitHub, with their latest workflow run, for those Quod has no data on. */
  private async githubRepos(g: GitHubState, have: Set<string>): Promise<RepoSummary[]> {
    if (g.teamId === null || g.link.state !== "linked") return [];
    try {
      const { recentRuns } = await live();
      const runs = await recentRuns(g.teamId, null, { limit: 200, db: g.db });
      return g.repos
        .filter((r) => !have.has(r.slug))
        .map((r) => {
          const latest = runs.find((x) => x.repoName.toLowerCase() === r.fullName.toLowerCase()) ?? null;
          return {
            slug: r.slug,
            fullName: r.fullName,
            language: "GitHub",
            runner: "github-hosted" as const,
            replayRate: 0,
            savedMinutes: 0,
            lastRunAt: latest?.startedAt ?? latest?.updatedAt ?? null,
            lastRunStatus: latest ? (latest.status !== "completed" ? "running" : latest.githubConclusion === "success" ? "passed" : "failed") : null,
            pausedSteps: 0,
            hasQuodData: false,
            github: {
              latestRun: latest
                ? { status: latest.status, conclusion: latest.githubConclusion, title: latest.title, url: latest.githubUrl, at: latest.startedAt ?? latest.updatedAt }
                : null,
            },
          } satisfies RepoSummary;
        });
    } catch (e) {
      console.warn("[quod-cloud] github repos unavailable", { error: e instanceof Error ? e.name : "unknown" });
      return [];
    }
  }

  // ---------------------------------------------------------------- Live

  async getActivity(team: string, opts: { repo?: string } = {}): Promise<Activity> {
    const base = await this.inner.getActivity(team, opts);
    if (base.simulated) return base; // the sample team keeps the simulator
    const g = await this.github(team);
    if (g.teamId === null || g.link.state !== "linked") return { ...base, github: g.link };
    try {
      const { maxJobVersion, queuedJobs, runningJobs } = await live();
      // The cursor first: anything committed after it reaches the stream, even if the job lists below already show it.
      const version = await maxJobVersion(g.teamId, g.db);
      const [running, queued] = await Promise.all([runningJobs(g.teamId, g.db), queuedJobs(g.teamId, g.db)]);
      const jobs = [...queued, ...running]
        .filter((j) => !opts.repo || j.repoSlug === opts.repo)
        .map(fromGitHub)
        .sort((a, b) => a.queuedAt.localeCompare(b.queuedAt));
      return { ...base, jobs: [...base.jobs, ...jobs], cursor: joinCursor(`gh-${version}`, base.cursor), github: g.link };
    } catch (e) {
      console.warn("[quod-cloud] github activity unavailable", { error: e instanceof Error ? e.name : "unknown" });
      return { ...base, github: { state: "unavailable" } };
    }
  }

  /**
   * Merged stream: GitHub job events (from the team's shared poller) and the
   * inner adapter's events. Ids carry both positions (lib/live/cursor.ts).
   */
  async *subscribe(team: string, scope: LiveScope, opts: { lastEventId?: string | null; signal: AbortSignal }): AsyncIterable<LiveEvent> {
    const { gh, quod } = splitCursor(opts.lastEventId);
    const g = opts.lastEventId && !gh ? null : await this.githubForStream(team);
    if (!g) {
      // Not linked (or a sample/single-source cursor): the inner stream, ids unchanged.
      yield* this.inner.subscribe(team, scope, { lastEventId: opts.lastEventId, signal: opts.signal });
      return;
    }
    const { teamId, db } = g;
    const { onTeamJobChange, recentJobEvents } = await live();
    const resync = this.opts.resyncWindowMs ?? RESYNC_MS;
    const interval = this.opts.pollIntervalMs;

    async function* jobs(signal: AbortSignal): AsyncGenerator<LiveJobEvent> {
      // Changes that may have raced the snapshot: repeated on purpose, dropped below if already seen.
      if (gh) for (const e of await recentJobEvents(teamId, resync, db)) yield e;
      yield* onTeamJobChange(teamId, { signal, after: gh, db, ...(interval ? { intervalMs: interval, maxIntervalMs: interval } : {}) });
    }

    let lastGh = gh ?? "gh-0";
    let lastQuod = quod;
    const newest = new Map<string, number>(); // job id -> highest version sent: an older state never overwrites a newer one
    for await (const { index, value } of mergeAsync<LiveEvent | LiveJobEvent>(
      [jobs(opts.signal), this.inner.subscribe(team, scope, { lastEventId: quod, signal: opts.signal })],
      opts.signal,
    )) {
      if (index === 0) {
        const e = value as LiveJobEvent;
        const v = ghVersion(e.id);
        if ((newest.get(e.job.id) ?? -1) >= v) continue;
        newest.set(e.job.id, v);
        if (v > ghVersion(lastGh)) lastGh = e.id;
        const ev = { ...e, job: fromGitHub(e.job), id: joinCursor(lastGh, lastQuod)! } as LiveEvent;
        if (inScope(ev, scope)) yield ev;
      } else {
        const e = value as LiveEvent;
        lastQuod = e.id;
        yield { ...e, id: joinCursor(lastGh, lastQuod)! };
      }
    }
  }

  private async githubForStream(team: string): Promise<{ teamId: TeamId; db: Db } | null> {
    const base = await this.inner.getActivity(team).catch(() => null);
    if (base?.simulated) return null;
    const g = await this.github(team);
    return g.teamId !== null && g.link.state === "linked" ? { teamId: g.teamId, db: g.db } : null;
  }

  // ---------------------------------------------------------------- Repos: GitHub's next to Quod's

  async getOverview(team: string, range: Parameters<CloudApi["getOverview"]>[1]): Promise<Overview> {
    const ov = await this.inner.getOverview(team, range);
    const g = await this.github(team);
    const extra = await this.githubRepos(g, new Set(ov.repos.map((r) => r.slug)));
    return extra.length ? { ...ov, repos: [...ov.repos, ...extra] } : ov;
  }

  async listRepos(team: string): Promise<RepoSummary[]> {
    const repos = await this.inner.listRepos(team);
    const g = await this.github(team);
    return [...repos, ...(await this.githubRepos(g, new Set(repos.map((r) => r.slug))))];
  }

  async getRepo(team: string, repo: string): Promise<RepoDetail | null> {
    const own = await this.inner.getRepo(team, repo);
    if (own) return own;
    const g = await this.github(team);
    const summary = (await this.githubRepos(g, new Set())).find((r) => r.slug === repo);
    if (!summary) return null;
    return { asOf: new Date().toISOString(), summary, defaultBranch: "main", latestRun: null, runTimes: [], steps: [] };
  }

  // ---------------------------------------------------------------- Everything else: the inner adapter

  resolveTeams: CloudApi["resolveTeams"] = (orgs) => this.inner.resolveTeams(orgs);
  getTeamHealth: CloudApi["getTeamHealth"] = (team) => this.inner.getTeamHealth(team);
  listRuns: CloudApi["listRuns"] = (team, opts) => this.inner.listRuns(team, opts);
  getRun: CloudApi["getRun"] = (team, repo, run) => this.inner.getRun(team, repo, run);
  listSteps: CloudApi["listSteps"] = (team, opts) => this.inner.listSteps(team, opts);
  getStepTrust: CloudApi["getStepTrust"] = (team, repo, key) => this.inner.getStepTrust(team, repo, key);
  setProofsNeeded: CloudApi["setProofsNeeded"] = (team, repo, key, need) => this.inner.setProofsNeeded(team, repo, key, need);
  requestRerunWithoutQuod: CloudApi["requestRerunWithoutQuod"] = (team, repo, run) => this.inner.requestRerunWithoutQuod(team, repo, run);
  listRunners: CloudApi["listRunners"] = (team) => this.inner.listRunners(team);
  getConnectSetup: CloudApi["getConnectSetup"] = (team) => this.inner.getConnectSetup(team);
  saveConnectChoice: CloudApi["saveConnectChoice"] = (team, choice) => this.inner.saveConnectChoice(team, choice);
  getSearchIndex: CloudApi["getSearchIndex"] = (team) => this.inner.getSearchIndex(team);
}
