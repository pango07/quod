// One URL slug per repository, whichever source knows it.
//
// quod-server and the GitHub App each make a repo slug, by different rules
// (quod-server lowercases and turns dots into dashes, and adds the owner for a
// reserved page name; the dashboard keeps GitHub's name and leaves reserved
// ones out), over different sets of repos. The same repository is the same
// `owner/name`, so the two are matched by full name (case-insensitive, as
// GitHub's are), and the dashboard shows the GitHub slug. The composite maps
// every repo slug going to quod-server and coming back through this.

import { TEAM_PAGES } from "@/lib/routes";
import type { LiveEvent, LiveScope, Overview, RepoDetail, RepoSummary, RunDetail, RunSummary, SearchIndex, StepSummary, StepTrust, TestRunDetail, TestRunSummary, TestStats } from "./types";

export interface RepoMap {
  /** Dashboard slug -> quod-server's slug (unknown slugs pass through). */
  toQuod(slug: string): string;
  /**
   * Dashboard slug -> quod-server's slug, or null when the slug is a GitHub
   * repo quod-server has no data for: never ask then, or another owner's repo
   * of the same name would answer for it.
   */
  quodSlug(slug: string): string | null;
  /** quod-server's slug -> dashboard slug (unknown slugs pass through). */
  toDash(slug: string): string;
  /** quod-server's repo by full name (lowercase). */
  quodByName: Map<string, { slug: string; fullName: string }>;
  identity: boolean;
  /** quod-server's repo list could not be read: the map is GitHub's alone. */
  failed?: Error;
}

export const IDENTITY: RepoMap = { toQuod: (s) => s, quodSlug: (s) => s, toDash: (s) => s, quodByName: new Map(), identity: true };

/** No GitHub link: quod-server's slugs as they are, except one a team page owns (/[team]/events is the stream). */
function identityFor(quod: { slug: string; fullName: string }[]): RepoMap {
  const q2d = new Map<string, string>();
  const d2q = new Map<string, string>();
  for (const r of quod) {
    if (!TEAM_PAGES.includes(lower(r.slug)) && lower(r.slug) !== "events") continue;
    const dash = r.fullName.replace("/", "-");
    q2d.set(r.slug, dash);
    d2q.set(lower(dash), r.slug);
  }
  const toQuod = (s: string) => d2q.get(lower(s)) ?? s;
  return { toQuod, quodSlug: toQuod, toDash: (s) => q2d.get(s) ?? s, quodByName: new Map(quod.map((r) => [lower(r.fullName), r])), identity: q2d.size === 0 };
}

const lower = (s: string) => s.toLowerCase();

export function buildRepoMap(quod: { slug: string; fullName: string }[], github: { slug: string; fullName: string }[]): RepoMap {
  if (!github.length) return identityFor(quod);
  const ghByName = new Map(github.map((r) => [lower(r.fullName), r.slug]));
  const taken = new Set(github.map((r) => lower(r.slug)));
  const q2d = new Map<string, string>();
  const d2q = new Map<string, string>();
  for (const r of quod) {
    let dash = ghByName.get(lower(r.fullName));
    if (!dash) {
      // Only Quod knows it (the App does not cover it): its own slug, unless a GitHub repo already shows under that one.
      dash = r.slug;
      if (taken.has(lower(dash)) || TEAM_PAGES.includes(lower(dash)) || lower(dash) === "events") dash = r.fullName.replace("/", "-");
    }
    taken.add(lower(dash));
    q2d.set(r.slug, dash);
    d2q.set(lower(dash), r.slug);
  }
  const ghSlugs = new Set(github.map((r) => lower(r.slug)));
  return {
    toQuod: (s) => d2q.get(lower(s)) ?? s,
    quodSlug: (s) => d2q.get(lower(s)) ?? (ghSlugs.has(lower(s)) ? null : s),
    toDash: (s) => q2d.get(s) ?? s,
    quodByName: new Map(quod.map((r) => [lower(r.fullName), r])),
    identity: false,
  };
}

// ---------------------------------------------------------------- Rewriting quod-server's answers

export const mapRepoSummary = (m: RepoMap, r: RepoSummary): RepoSummary => (m.identity ? r : { ...r, slug: m.toDash(r.slug) });
export const mapRunSummary = <R extends RunSummary | RunDetail>(m: RepoMap, r: R): R => (m.identity ? r : { ...r, repoSlug: m.toDash(r.repoSlug) });
export const mapStep = <S extends StepSummary | StepTrust>(m: RepoMap, s: S): S => (m.identity ? s : { ...s, repoSlug: m.toDash(s.repoSlug) });

export const mapTestRun = <T extends TestRunSummary>(m: RepoMap, r: T): T => (m.identity ? r : { ...r, repoSlug: m.toDash(r.repoSlug) });

export function mapTestRunDetail(m: RepoMap, d: TestRunDetail): TestRunDetail {
  if (m.identity) return d;
  return { ...mapTestRun(m, d), shadow: d.shadow ? mapTestRun(m, d.shadow) : null };
}

export function mapTestStats(m: RepoMap, s: TestStats): TestStats {
  return m.identity ? s : { ...s, recent: s.recent.map((r) => mapTestRun(m, r)) };
}

export function mapOverview(m: RepoMap, o: Overview): Overview {
  if (m.identity) return o;
  return { ...o, repos: o.repos.map((r) => mapRepoSummary(m, r)), trust: { ...o.trust, paused: o.trust.paused.map((p) => ({ ...p, repoSlug: m.toDash(p.repoSlug) })) } };
}

export function mapRepoDetail(m: RepoMap, d: RepoDetail): RepoDetail {
  if (m.identity) return d;
  return { ...d, summary: mapRepoSummary(m, d.summary), latestRun: d.latestRun ? mapRunSummary(m, d.latestRun) : null, steps: d.steps.map((s) => mapStep(m, s)) };
}

export function mapSearchIndex(m: RepoMap, i: SearchIndex): SearchIndex {
  if (m.identity) return i;
  return {
    repos: i.repos.map((r) => ({ ...r, slug: m.toDash(r.slug) })),
    runs: i.runs.map((r) => ({ ...r, repoSlug: m.toDash(r.repoSlug) })),
    steps: i.steps.map((s) => ({ ...s, repoSlug: m.toDash(s.repoSlug) })),
  };
}

export function mapEvent(m: RepoMap, e: LiveEvent): LiveEvent {
  if (m.identity) return e;
  switch (e.type) {
    case "job.queued":
    case "job.started":
    case "job.finished":
      return { ...e, job: { ...e.job, repoSlug: m.toDash(e.job.repoSlug) } };
    case "run.started":
    case "run.finished":
      return { ...e, run: mapRunSummary(m, e.run) };
    case "test.run":
      return { ...e, repoSlug: m.toDash(e.repoSlug), run: mapTestRun(m, e.run) };
    default:
      return { ...e, repoSlug: m.toDash(e.repoSlug) };
  }
}

export function scopeToQuod(m: RepoMap, s: LiveScope): LiveScope {
  return s.kind === "team" || m.identity ? s : { ...s, repo: m.toQuod(s.repo) };
}
