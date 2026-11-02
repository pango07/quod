// CloudApi composite: CI job activity from the GitHub App, Quod's own data
// (runs, steps, trust, stats) from the inner adapter (fixture, HTTP, or none).
//
// Which data a team sees is decided once per request from its WorkOS org id,
// never from its slug or name (anyone can name an org "quodci"):
// - The sample team (the demo session's sample org, `isSample`): the inner
//   adapter only, sample data and the simulator. Never GitHub data.
// - Every other team: the inner adapter's (real) data, plus, when a GitHub
//   installation is linked, its repos with their latest workflow run and
//   default-branch protection, and its live jobs ("Now running", job.*
//   events) from Postgres (lib/github/live.ts).
//   Not linked: Activity.github says so, and pages show the Connect card.
// The two sources meet per repository by full name (owner/name), never by
// slug: quod-server's slugs are mapped to the dashboard's (repo-map.ts). A
// repo Quod has data for shows Quod's numbers; one it has not, GitHub's run
// and "Appears once Quod runs".
//
// Failures: without a database, or when it does not answer, the inner
// adapter's answers stand and Activity.github says why. When quod-server does
// not answer (ApiUnavailableError), the pages that can show GitHub's data
// show it alone, with `quod: { state: "unavailable" }` for a note; the others
// get the error and render a section card (components/section.tsx).

import { unstable_rethrow } from "next/navigation";
import type { Db } from "@/lib/db/types";
import { teamIdOf, type TeamId } from "@/lib/github/team";
import type { LiveJobEvent } from "@/lib/live-events";
import { joinCursor, splitCursor } from "@/lib/live/cursor";
import { mergeAsync } from "@/lib/live/merge";
import { inScope } from "@/lib/live/scope";
import { isSlug } from "@/lib/params";
import { TEAM_PAGES } from "@/lib/routes";
import { NoQuodCloudApi } from "./none";
import { buildRepoMap, IDENTITY, mapEvent, mapOverview, mapRepoDetail, mapRepoSummary, mapRunSummary, mapSearchIndex, mapStep, scopeToQuod, type RepoMap } from "./repo-map";
import { ApiNotAvailableError, ApiUnavailableError } from "./types";
import type { Activity, BranchProtection, CloudApi, GitHubLink, LiveEvent, LiveJob, LiveScope, Overview, QuodStatus, RepoDetail, RepoSummary, SearchIndex } from "./types";

/** What the composite needs from the GitHub side (lib/github/live.ts and friends); injectable for tests. */
export interface GitHubSource {
  /** Is there a database to read? (DATABASE_URL set.) */
  configured(): boolean;
  /** Is the GitHub App set up on this deploy (so admins can install it)? */
  appReady(): boolean;
  db(): Promise<Db>;
  /** Default-branch protection per GitHub repo id, from the cache (lib/github/protection.ts). Absent, or null (no cache table yet): not shown. */
  protection?(teamId: TeamId, db: Db, repos: GhRepo[]): Promise<Map<number, BranchProtection> | null>;
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
  /** Where a Quod failure is reported (default: a console warning with the error's name only). */
  onQuodError?: (what: string, e: unknown) => void;
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

const warn = (what: string, e: unknown) => console.warn(`[quod-cloud] ${what}`, { error: e instanceof Error ? e.name : "unknown", message: e instanceof ApiUnavailableError ? e.message : undefined });

/** quod-server could not be used: show what GitHub has, and say so. */
const unavailable = (e: ApiUnavailableError): QuodStatus => ({ state: "unavailable", message: e.message });

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

interface GhRepoInfo {
  summary: RepoSummary;
  repo: GhRepo;
}

export class CompositeCloudApi implements CloudApi {
  private readonly ctxs = new Map<string, Promise<TeamCtx>>();
  private readonly ghStates = new Map<string, Promise<GitHubState>>();
  private readonly ghInfos = new Map<string, Promise<GhRepoInfo[]>>();
  private readonly maps = new Map<string, Promise<RepoMap>>();
  private readonly quodRepos = new Map<string, Promise<RepoSummary[]>>();
  private readonly none = new NoQuodCloudApi();

  constructor(
    private readonly inner: CloudApi,
    private readonly opts: CompositeOptions,
  ) {}

  /** quod-server's repo list, once per instance (the slug map and the repo list share it). A failure is not remembered. */
  private innerRepos(team: string): Promise<RepoSummary[]> {
    let p = this.quodRepos.get(team);
    if (!p) {
      p = this.inner.listRepos(team);
      p.catch(() => this.quodRepos.delete(team));
      this.quodRepos.set(team, p);
    }
    return p;
  }

  private quodError(what: string, e: unknown) {
    (this.opts.onQuodError ?? warn)(what, e);
  }

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

  /** The team's GitHub status, and its repos when linked; once per instance. Never throws, except Next's redirect/notFound. */
  private github(c: TeamCtx): Promise<GitHubState> {
    const key = c.orgId ?? "";
    let g = this.ghStates.get(key);
    if (!g) {
      g = readGitHubState(c.orgId, this.opts.github);
      g.catch(() => this.ghStates.delete(key));
      this.ghStates.set(key, g);
    }
    return g;
  }

  /** The linked repos as RepoSummaries (GitHub's latest run, protection, no Quod numbers). Never throws. */
  private ghRepoInfos(g: GitHubState): Promise<GhRepoInfo[]> {
    if (g.teamId === null || g.link.state !== "linked") return Promise.resolve([]);
    const { teamId, db, repos } = g;
    let p = this.ghInfos.get(teamId);
    if (!p) {
      p = (async () => {
        try {
          const { latestRunsByRepo } = await live();
          const [latestByRepo, protection] = await Promise.all([
            latestRunsByRepo(teamId, db),
            this.opts.github.protection ? this.opts.github.protection(teamId, db, repos).catch((e) => (warn("branch protection unavailable", e), null)) : Promise.resolve(null),
          ]);
          return repos.map((r) => {
            const latest = latestByRepo.get(r.id) ?? null;
            const summary: RepoSummary = {
              slug: r.slug,
              fullName: r.fullName,
              language: "GitHub",
              runner: "github-hosted",
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
              ...(protection ? { protection: protection.get(r.id) ?? { state: "unknown", branch: null, checkedAt: null } } : {}),
            };
            return { summary, repo: r };
          });
        } catch (e) {
          unstable_rethrow(e);
          warn("github repos unavailable", e);
          return [];
        }
      })();
      this.ghInfos.set(teamId, p);
    }
    return p;
  }

  /**
   * quod-server's slugs to the dashboard's, for a linked team (identity
   * otherwise, and for the sample team). Reads quod-server's repo list once
   * per instance; when it cannot, slugs pass through unchanged.
   */
  private repoMap(team: string, c: TeamCtx): Promise<RepoMap> {
    if (c.simulated) return Promise.resolve(IDENTITY);
    let m = this.maps.get(team);
    if (!m) {
      m = (async () => {
        const g = await this.github(c);
        if (g.teamId === null || g.link.state !== "linked") return IDENTITY;
        try {
          return buildRepoMap(await this.innerRepos(team), g.repos);
        } catch (e) {
          unstable_rethrow(e);
          if (!(e instanceof ApiUnavailableError || e instanceof ApiNotAvailableError)) throw e;
          return { ...buildRepoMap([], g.repos), failed: e instanceof ApiUnavailableError ? e : undefined };
        }
      })();
      m.catch(() => this.maps.delete(team));
      this.maps.set(team, m);
    }
    return m;
  }

  /** Quod's repos and GitHub's, one per full name: Quod's numbers, GitHub's latest run and protection next to them. */
  private mergeRepos(quod: RepoSummary[], gh: GhRepoInfo[]): RepoSummary[] {
    const ghBy = new Map(gh.map((x) => [lower(x.summary.fullName), x.summary]));
    const merged = quod.map((r) => {
      const g = ghBy.get(lower(r.fullName));
      if (!g) return r;
      ghBy.delete(lower(r.fullName));
      return { ...r, ...(g.github ? { github: g.github } : {}), ...(g.protection ? { protection: g.protection } : {}) };
    });
    return [...merged, ...ghBy.values()];
  }

  // ---------------------------------------------------------------- Live

  async getActivity(team: string, opts: { repo?: string } = {}): Promise<Activity> {
    const c = await this.ctx(team);
    if (c.simulated) return this.inner.getActivity(team, opts);
    const map = await this.repoMap(team, c);
    let base: Activity;
    try {
      base = await this.inner.getActivity(team, opts.repo ? { repo: map.toQuod(opts.repo) } : {});
      base = { ...base, jobs: base.jobs.map((j) => ({ ...j, repoSlug: map.toDash(j.repoSlug) })) };
    } catch (e) {
      unstable_rethrow(e);
      if (!(e instanceof ApiUnavailableError)) throw e;
      this.quodError("quod activity unavailable", e);
      base = { ...(await this.none.getActivity()), quod: unavailable(e) };
    }
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
   * Quod's events with the dashboard's repo slugs. A failure of quod-server's
   * stream ends only Quod's part (the adapter already reconnects on its own);
   * GitHub's keeps going, and the browser's next reconnect tries Quod again.
   */
  private async *quodEvents(team: string, scope: LiveScope, map: RepoMap, lastEventId: string | null, signal: AbortSignal): AsyncGenerator<LiveEvent> {
    try {
      for await (const e of this.inner.subscribe(team, scopeToQuod(map, scope), { lastEventId, signal })) yield mapEvent(map, e);
    } catch (e) {
      unstable_rethrow(e);
      if (signal.aborted) return;
      this.quodError("quod event stream ended", e);
    }
  }

  /**
   * Merged stream: GitHub job events (from the team's shared poller) and the
   * inner adapter's events. Ids carry both positions (lib/live/cursor.ts).
   * The inner adapter only ever sees its own part of the cursor.
   */
  async *subscribe(team: string, scope: LiveScope, opts: { lastEventId?: string | null; signal: AbortSignal }): AsyncIterable<LiveEvent> {
    const { gh, quod } = splitCursor(opts.lastEventId);
    const c = await this.ctx(team);
    if (c.simulated) {
      yield* this.inner.subscribe(team, scope, { lastEventId: quod, signal: opts.signal });
      return;
    }
    const map = await this.repoMap(team, c);
    const innerStream = () => this.quodEvents(team, scope, map, quod, opts.signal);
    let setup: Awaited<ReturnType<CompositeCloudApi["streamSetup"]>> = null;
    try {
      setup = await this.streamSetup(c, gh);
    } catch (e) {
      unstable_rethrow(e);
      warn("github stream unavailable", e); // the inner stream alone; the browser's next reconnect tries again
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
    if (c.simulated) return this.inner.getOverview(team, range);
    const [map, g] = await Promise.all([this.repoMap(team, c), this.github(c)]);
    let ov: Overview;
    try {
      ov = mapOverview(map, await this.inner.getOverview(team, range));
    } catch (e) {
      unstable_rethrow(e);
      if (!(e instanceof ApiUnavailableError)) throw e;
      this.quodError("quod overview unavailable", e);
      ov = { ...(await this.none.getOverview(team, range)), quod: unavailable(e) };
    }
    const gh = await this.ghRepoInfos(g);
    return gh.length ? { ...ov, repos: this.mergeRepos(ov.repos, gh) } : ov;
  }

  async listRepos(team: string): Promise<RepoSummary[]> {
    const c = await this.ctx(team);
    if (c.simulated) return this.inner.listRepos(team);
    const [map, g] = await Promise.all([this.repoMap(team, c), this.github(c)]);
    let repos: RepoSummary[] = [];
    try {
      repos = (await this.innerRepos(team)).map((r) => mapRepoSummary(map, r));
    } catch (e) {
      unstable_rethrow(e);
      if (!(e instanceof ApiUnavailableError)) throw e;
      this.quodError("quod repos unavailable", e);
    }
    return this.mergeRepos(repos, await this.ghRepoInfos(g));
  }

  async getRepo(team: string, repo: string): Promise<RepoDetail | null> {
    const c = await this.ctx(team);
    if (c.simulated) return this.inner.getRepo(team, repo);
    const [map, g] = await Promise.all([this.repoMap(team, c), this.github(c)]);
    const gh = (await this.ghRepoInfos(g)).find((x) => lower(x.repo.slug) === lower(repo));
    let own: RepoDetail | null = null;
    let status: QuodStatus | undefined;
    try {
      if (map.failed && gh) throw map.failed;
      // A GitHub repo Quod's repo list does not have: no need to ask.
      const quodSlug = map.toQuod(repo);
      const known = map.identity || !gh || map.quodByName.has(lower(gh.summary.fullName));
      own = known ? await this.inner.getRepo(team, quodSlug) : null;
    } catch (e) {
      unstable_rethrow(e);
      if (!(e instanceof ApiUnavailableError) || !gh) throw e;
      this.quodError("quod repo unavailable", e);
      status = unavailable(e);
    }
    if (own) {
      const d = mapRepoDetail(map, own);
      return gh ? { ...d, summary: this.mergeRepos([d.summary], [gh])[0] } : d;
    }
    if (!gh) return null;
    return { asOf: new Date().toISOString(), summary: gh.summary, defaultBranch: gh.summary.protection?.branch ?? "main", latestRun: null, runTimes: [], steps: [], ...(status ? { quod: status } : {}) };
  }

  async listRuns(team: string, opts: { repo?: string; limit?: number } = {}) {
    const c = await this.ctx(team);
    if (c.simulated) return this.inner.listRuns(team, opts);
    const map = await this.repoMap(team, c);
    try {
      const r = await this.inner.listRuns(team, { ...opts, ...(opts.repo ? { repo: map.toQuod(opts.repo) } : {}) });
      return { ...r, runs: r.runs.map((x) => mapRunSummary(map, x)) };
    } catch (e) {
      unstable_rethrow(e);
      if (!(e instanceof ApiUnavailableError)) throw e;
      this.quodError("quod runs unavailable", e);
      return { asOf: new Date().toISOString(), runs: [], quod: unavailable(e) };
    }
  }

  async getRun(team: string, repo: string, run: number) {
    const c = await this.ctx(team);
    if (c.simulated) return this.inner.getRun(team, repo, run);
    const map = await this.repoMap(team, c);
    const r = await this.inner.getRun(team, map.toQuod(repo), run);
    return r ? mapRunSummary(map, r) : null;
  }

  async listSteps(team: string, opts: { repo?: string } = {}) {
    const c = await this.ctx(team);
    if (c.simulated) return this.inner.listSteps(team, opts);
    const map = await this.repoMap(team, c);
    const steps = await this.inner.listSteps(team, opts.repo ? { repo: map.toQuod(opts.repo) } : {});
    return steps.map((s) => mapStep(map, s));
  }

  async getStepTrust(team: string, repo: string, key: string) {
    const c = await this.ctx(team);
    if (c.simulated) return this.inner.getStepTrust(team, repo, key);
    const map = await this.repoMap(team, c);
    const s = await this.inner.getStepTrust(team, map.toQuod(repo), key);
    return s ? mapStep(map, s) : null;
  }

  async setProofsNeeded(team: string, repo: string, key: string, need: number) {
    const c = await this.ctx(team);
    const map = await this.repoMap(team, c);
    return mapStep(map, await this.inner.setProofsNeeded(team, map.toQuod(repo), key, need));
  }

  async requestRerunWithoutQuod(team: string, repo: string, run: number) {
    const c = await this.ctx(team);
    const map = await this.repoMap(team, c);
    return this.inner.requestRerunWithoutQuod(team, map.toQuod(repo), run);
  }

  /** quod-server answers 501 for runners (GitHub's "Seen in your CI" shows them): none from Quod. */
  async listRunners(team: string) {
    try {
      return await this.inner.listRunners(team);
    } catch (e) {
      unstable_rethrow(e);
      if (e instanceof ApiNotAvailableError) return [];
      throw e;
    }
  }

  /** The setup instructions are the same for every team; when quod-server does not keep them (501, or down), the built-in ones. */
  async getConnectSetup(team: string) {
    const c = await this.ctx(team);
    try {
      const s = await this.inner.getConnectSetup(team);
      if (c.simulated) return s;
      const map = await this.repoMap(team, c);
      return s.firstRepo ? { ...s, firstRepo: { ...s.firstRepo, slug: map.toDash(s.firstRepo.slug) } } : s;
    } catch (e) {
      unstable_rethrow(e);
      if (!(e instanceof ApiNotAvailableError || e instanceof ApiUnavailableError)) throw e;
      const first = (await this.ghRepoInfos(await this.github(c)))[0]?.summary;
      return { ...(await this.none.getConnectSetup()), firstRepo: first ? { slug: first.slug, fullName: first.fullName } : null };
    }
  }

  /** The ⌘K menu: Quod's runs and steps, and every repo from either source. */
  async getSearchIndex(team: string): Promise<SearchIndex> {
    const c = await this.ctx(team);
    if (c.simulated) return this.inner.getSearchIndex(team);
    const [map, g] = await Promise.all([this.repoMap(team, c), this.github(c)]);
    let idx: SearchIndex = { repos: [], runs: [], steps: [] };
    try {
      idx = mapSearchIndex(map, await this.inner.getSearchIndex(team));
    } catch (e) {
      unstable_rethrow(e);
      if (!(e instanceof ApiUnavailableError || e instanceof ApiNotAvailableError)) throw e;
    }
    const have = new Set(idx.repos.map((r) => lower(r.fullName)));
    const extra = (await this.ghRepoInfos(g)).filter((x) => !have.has(lower(x.summary.fullName))).map((x) => ({ slug: x.summary.slug, fullName: x.summary.fullName }));
    return extra.length ? { ...idx, repos: [...idx.repos, ...extra] } : idx;
  }

  /** The sidebar on every page: zeros rather than an error when quod-server is down or does not have it. */
  async getTeamHealth(team: string) {
    try {
      return await this.inner.getTeamHealth(team);
    } catch (e) {
      unstable_rethrow(e);
      if (!(e instanceof ApiUnavailableError || e instanceof ApiNotAvailableError)) throw e;
      return this.none.getTeamHealth();
    }
  }

  // ---------------------------------------------------------------- The inner adapter as is

  resolveTeams: CloudApi["resolveTeams"] = (orgs) => this.inner.resolveTeams(orgs);
  saveConnectChoice: CloudApi["saveConnectChoice"] = (team, choice) => this.inner.saveConnectChoice(team, choice);
}
