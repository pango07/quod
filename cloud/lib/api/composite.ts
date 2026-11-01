// CloudApi composite: CI job activity from the GitHub App, everything else
// (Quod's runs, steps, trust, stats) from the inner adapter (fixture or HTTP).
//
// Which data a team sees is decided once per request from its WorkOS org id,
// never from its slug or name (anyone can name an org "quodci"):
// - The sample team (the demo session's sample org, `isSample`): the inner
//   adapter only, sample data and the simulator. Never GitHub data.
// - Every other team: the inner adapter's (real) data, plus, when a GitHub
//   installation is linked, its repos with their latest workflow run and its
//   live jobs ("Now running", job.* events) from Postgres (lib/github/live.ts).
//   Not linked: Activity.github says so, and pages show the Connect card.
// Without a database, or when it does not answer, the inner adapter's
// answers stand and Activity.github says why (pages render a notice).

import { unstable_rethrow } from "next/navigation";
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
  /** Is this org the sample team (sample data, simulator)? By org id only. Default: none is. */
  isSample?: (orgId: string) => boolean;
  /** How far back a new stream replays GitHub job changes, to cover commits that raced its snapshot. */
  resyncWindowMs?: number;
  /** A fixed poll interval for the job feed, no idle back-off (tests). Default: the poller's own (2 s, backing off to 10 s). */
  pollIntervalMs?: number;
}

export interface GhRepo {
  id: number;
  fullName: string;
  slug: string;
}

export type GitHubState =
  | { link: GitHubLink; teamId: null }
  | { link: Extract<GitHubLink, { state: "linked" }> | Extract<GitHubLink, { state: "unlinked" }>; teamId: TeamId; db: Db; repos: GhRepo[] };

interface TeamCtx {
  orgId: string | null;
  /** The sample team: the inner adapter alone. */
  simulated: boolean;
}

const RESYNC_MS = 30_000;
const live = () => import("@/lib/github/live");

/** GitHub numbers steps from 1; LiveJob.currentStep.index counts from 0 (like Quod's step events). */
function fromGitHub<J extends LiveJob>(job: J): J {
  return job.currentStep ? { ...job, currentStep: { ...job.currentStep, index: Math.max(0, job.currentStep.index - 1) } } : job;
}

const ghVersion = (id: string | null) => (id && /^gh-(\d{1,15})$/.test(id) ? Number(id.slice(3)) : 0);
const lower = (s: string) => s.toLowerCase();

const warn = (what: string, e: unknown) => console.warn(`[quod-cloud] ${what}`, { error: e instanceof Error ? e.name : "unknown" });

/**
 * Repo slugs for an installation's repos: the bare name, or "owner-name" when
 * two linked owners have a repo of that name. Keyed by repo id and full name,
 * never by bare name. A slug that is not usable, or that a team page owns, is left out.
 */
export function githubRepoSlugs(repos: { id: number; fullName: string }[]): GhRepo[] {
  const byName = new Map<string, number>();
  const unique = repos.filter((r, i) => repos.findIndex((x) => x.id === r.id) === i);
  for (const r of unique) {
    const name = lower(r.fullName.split("/")[1] ?? "");
    byName.set(name, (byName.get(name) ?? 0) + 1);
  }
  const seen = new Set<string>();
  const out: GhRepo[] = [];
  for (const r of unique) {
    const [owner = "", name = ""] = r.fullName.split("/");
    const slug = (byName.get(lower(name)) ?? 0) > 1 ? `${owner}-${name}` : name;
    if (!isSlug(slug) || TEAM_PAGES.includes(lower(slug)) || seen.has(lower(slug))) continue;
    seen.add(lower(slug));
    out.push({ id: r.id, fullName: r.fullName, slug });
  }
  return out;
}

/** A GitHub job under its slug on this dashboard (found by full name), steps from 0. */
function asOurs<J extends LiveJob>(job: J, repos: GhRepo[]): J {
  const mine = repos.find((r) => lower(r.fullName) === lower(job.repoName));
  return fromGitHub(mine ? { ...job, repoSlug: mine.slug } : job);
}

/**
 * A team's GitHub status by org id, and its repos when linked. Never throws,
 * except Next's redirect/notFound. Shared by the composite and the GitHub
 * pages (lib/github/team-data.ts), so both see the same link.
 */
export async function readGitHubState(orgId: string | null, gh: GitHubSource): Promise<GitHubState> {
  if (!gh.configured()) return { link: { state: "not-configured" }, teamId: null };
  if (!orgId) return { link: { state: "unlinked", canInstall: false }, teamId: null };
  try {
    const teamId = teamIdOf({ team: { orgId } });
    const db = await gh.db();
    const { teamInstallations } = await live();
    const installs = await teamInstallations(teamId, db);
    const active = installs.filter((i) => !i.suspended);
    if (!active.length) return { link: { state: "unlinked", canInstall: gh.appReady() }, teamId, db, repos: [] };
    const repos = githubRepoSlugs(active.flatMap((i) => i.repos));
    return { link: { state: "linked", repos: repos.length, accounts: [...new Set(active.map((i) => i.account).filter(Boolean))] }, teamId, db, repos };
  } catch (e) {
    unstable_rethrow(e);
    warn("github source unavailable", e);
    return { link: { state: "unavailable" }, teamId: null };
  }
}

export class CompositeCloudApi implements CloudApi {
  private readonly ctxs = new Map<string, Promise<TeamCtx>>();

  constructor(
    private readonly inner: CloudApi,
    private readonly opts: CompositeOptions,
  ) {}

  /** The team's org, and whether it is the sample team: decided once per request, from the org id alone. */
  private ctx(team: string): Promise<TeamCtx> {
    let c = this.ctxs.get(team);
    if (!c) {
      c = this.opts.orgOf(team).then((orgId) => ({ orgId, simulated: orgId !== null && (this.opts.isSample?.(orgId) ?? false) }));
      // A failed lookup (sign-in redirect, outage) is not remembered.
      c.catch(() => this.ctxs.delete(team));
      this.ctxs.set(team, c);
    }
    return c;
  }

  /** The team's GitHub status, and its repos when linked. Never throws, except Next's redirect/notFound. */
  private github(c: TeamCtx): Promise<GitHubState> {
    return readGitHubState(c.orgId, this.opts.github);
  }

  /** Repos from GitHub, with their latest workflow run, for those Quod has no data on. */
  private async githubRepos(g: GitHubState, have: Set<string>): Promise<RepoSummary[]> {
    if (g.teamId === null || g.link.state !== "linked") return [];
    try {
      const { latestRunsByRepo } = await live();
      const latestByRepo = await latestRunsByRepo(g.teamId, g.db);
      return g.repos
        .filter((r) => !have.has(lower(r.slug)))
        .map((r) => {
          const latest = latestByRepo.get(r.id) ?? null;
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
                ? { id: latest.id, status: latest.status, conclusion: latest.githubConclusion, title: latest.title, url: latest.githubUrl, at: latest.startedAt ?? latest.updatedAt }
                : null,
            },
          } satisfies RepoSummary;
        });
    } catch (e) {
      unstable_rethrow(e);
      warn("github repos unavailable", e);
      return [];
    }
  }

  // ---------------------------------------------------------------- Live

  async getActivity(team: string, opts: { repo?: string } = {}): Promise<Activity> {
    const c = await this.ctx(team);
    const base = await this.inner.getActivity(team, opts);
    if (c.simulated) return base; // the sample team keeps the simulator
    const g = await this.github(c);
    if (g.teamId === null || g.link.state !== "linked") return { ...base, github: g.link };
    try {
      const { maxJobVersion, queuedJobs, runningJobs } = await live();
      // The cursor first: anything committed after it reaches the stream, even if the job lists below already show it.
      const version = await maxJobVersion(g.teamId, g.db);
      // One repo's jobs: matched by full name (two owners may share a bare name).
      const want = opts.repo ? g.repos.find((r) => r.slug === opts.repo) : undefined;
      const [running, queued] = opts.repo && !want ? [[], []] : await Promise.all([runningJobs(g.teamId, g.db), queuedJobs(g.teamId, g.db)]);
      const jobs = [...queued, ...running]
        .filter((j) => !want || lower(j.repoName) === lower(want.fullName))
        .map((j) => asOurs(j, g.repos))
        .sort((a, b) => a.queuedAt.localeCompare(b.queuedAt));
      return { ...base, jobs: [...base.jobs, ...jobs], cursor: joinCursor(`gh-${version}`, base.cursor), github: g.link };
    } catch (e) {
      unstable_rethrow(e);
      warn("github activity unavailable", e);
      return { ...base, github: { state: "unavailable" } };
    }
  }

  /** What a stream needs from GitHub, or null when the team is not linked. */
  private async streamSetup(c: TeamCtx, gh: string | null) {
    const g = await this.github(c);
    if (g.teamId === null || g.link.state !== "linked") return null;
    const mod = await live();
    // A position (from the client or the snapshot) resumes there. None, or "gh-0" (nothing seen yet),
    // starts from the team's newest version now: live from here, never a replay of its whole history.
    const after = ghVersion(gh) > 0 ? gh! : `gh-${await mod.maxJobVersion(g.teamId, g.db)}`;
    return { teamId: g.teamId, db: g.db, repos: g.repos, after, mod };
  }

  /**
   * Merged stream: GitHub job events (from the team's shared poller) and the
   * inner adapter's events. Ids carry both positions (lib/live/cursor.ts).
   * The inner adapter only ever sees its own part of the cursor.
   */
  async *subscribe(team: string, scope: LiveScope, opts: { lastEventId?: string | null; signal: AbortSignal }): AsyncIterable<LiveEvent> {
    const { gh, quod } = splitCursor(opts.lastEventId);
    const c = await this.ctx(team);
    const innerStream = () => this.inner.subscribe(team, scope, { lastEventId: quod, signal: opts.signal });
    let setup: Awaited<ReturnType<CompositeCloudApi["streamSetup"]>> = null;
    if (!c.simulated) {
      try {
        setup = await this.streamSetup(c, gh);
      } catch (e) {
        unstable_rethrow(e);
        warn("github stream unavailable", e); // the inner stream alone; the browser's next reconnect tries again
      }
    }
    if (!setup) {
      yield* innerStream();
      return;
    }
    const { teamId, db, repos, after, mod } = setup;
    const resync = this.opts.resyncWindowMs ?? RESYNC_MS;
    const interval = this.opts.pollIntervalMs;

    async function* jobs(signal: AbortSignal): AsyncGenerator<LiveJobEvent> {
      // Changes that may have raced the snapshot: repeated on purpose, dropped below if already seen.
      if (gh) for (const e of await mod.recentJobEvents(teamId, resync, db)) yield e;
      yield* mod.onTeamJobChange(teamId, { signal, after, db, ...(interval ? { intervalMs: interval, maxIntervalMs: interval } : {}) });
    }

    let lastGh = after;
    let lastQuod = quod;
    const newest = new Map<string, number>(); // job id -> highest version sent: an older state never overwrites a newer one
    for await (const { index, value } of mergeAsync<LiveEvent | LiveJobEvent>([jobs(opts.signal), innerStream()], opts.signal)) {
      if (index === 0) {
        const e = value as LiveJobEvent;
        const v = ghVersion(e.id);
        if ((newest.get(e.job.id) ?? -1) >= v) continue;
        newest.set(e.job.id, v);
        if (v > ghVersion(lastGh)) lastGh = e.id;
        const ev = { ...e, job: asOurs(e.job, repos), id: joinCursor(lastGh, lastQuod)! } as LiveEvent;
        if (inScope(ev, scope)) yield ev;
      } else {
        const e = value as LiveEvent;
        lastQuod = e.id;
        yield { ...e, id: joinCursor(lastGh, lastQuod)! };
      }
    }
  }

  // ---------------------------------------------------------------- Repos: GitHub's next to Quod's (never the sample team's)

  async getOverview(team: string, range: Parameters<CloudApi["getOverview"]>[1]): Promise<Overview> {
    const c = await this.ctx(team);
    const ov = await this.inner.getOverview(team, range);
    if (c.simulated) return ov;
    const extra = await this.githubRepos(await this.github(c), new Set(ov.repos.map((r) => lower(r.slug))));
    return extra.length ? { ...ov, repos: [...ov.repos, ...extra] } : ov;
  }

  async listRepos(team: string): Promise<RepoSummary[]> {
    const c = await this.ctx(team);
    const repos = await this.inner.listRepos(team);
    if (c.simulated) return repos;
    return [...repos, ...(await this.githubRepos(await this.github(c), new Set(repos.map((r) => lower(r.slug)))))];
  }

  async getRepo(team: string, repo: string): Promise<RepoDetail | null> {
    const c = await this.ctx(team);
    const own = await this.inner.getRepo(team, repo);
    if (own || c.simulated) return own;
    const summary = (await this.githubRepos(await this.github(c), new Set())).find((r) => r.slug === repo);
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
