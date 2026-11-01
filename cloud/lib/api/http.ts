// ============================================================================
// STUB: the HTTP adapter for quod-server (protocol "quod.v1").
//
// quod-server is not built yet. These are the endpoints the dashboard expects
// it to serve; docs/api.md describes them. The code has never run against a
// server, so treat it as the proposal in TypeScript form. The Rust side
// implements the endpoints, then this adapter is tested against it.
//
// Every request carries the viewer's WorkOS access token. The server must
// authorize by the token's subject (the user) and that user's membership in
// the team's WorkOS Organization, not by the token's org_id alone: a viewer
// may open any of their teams by URL without switching the session first.
// ============================================================================

import { isSlug } from "@/lib/params";
import { ApiForbiddenError, ApiInvalidRequestError } from "./types";
import type {
  CloudApi,
  ConnectSetup,
  Host,
  OrgRef,
  Overview,
  Range,
  RepoDetail,
  RepoSummary,
  RunDetail,
  Runner,
  RunSummary,
  SearchIndex,
  StepSummary,
  StepTrust,
  Store,
  Team,
  TeamHealth,
} from "./types";

export const PROTOCOL = "quod.v1";

const seg = encodeURIComponent;

/** Team, repo and step-key path segments are checked, so nothing can walk the backend path. */
function id(v: string): string {
  if (!isSlug(v)) throw new ApiInvalidRequestError("Invalid path segment");
  return seg(v);
}

/** A run number is a positive integer; anything else never reaches a URL. */
function runSeg(run: number): string {
  if (!Number.isSafeInteger(run) || run < 1 || run > 999_999_999) throw new ApiInvalidRequestError("Invalid run number");
  return seg(String(run));
}

export class HttpCloudApi implements CloudApi {
  constructor(
    private readonly baseUrl: string,
    private readonly accessToken: () => Promise<string | undefined>,
  ) {}

  private async request<T>(method: "GET" | "POST" | "PUT", path: string, body?: unknown): Promise<T | null> {
    const token = await this.accessToken();
    const res = await fetch(new URL(`/v1${path}`, this.baseUrl), {
      method,
      headers: {
        accept: "application/json",
        "quod-protocol": PROTOCOL,
        ...(body === undefined ? {} : { "content-type": "application/json" }),
        ...(token ? { authorization: `Bearer ${token}` } : {}),
      },
      body: body === undefined ? undefined : JSON.stringify(body),
      cache: "no-store",
    });
    if (res.status === 404) return null;
    if (res.status === 401 || res.status === 403) throw new ApiForbiddenError(`quod-server refused ${method} ${path}`, res.status);
    if (!res.ok) throw new Error(`quod-server ${method} ${path}: ${res.status} ${res.statusText}`);
    if (res.status === 204) return null;
    return (await res.json()) as T;
  }

  private async must<T>(method: "GET" | "POST" | "PUT", path: string, body?: unknown): Promise<T> {
    const v = await this.request<T>(method, path, body);
    if (v === null) throw new Error(`quod-server ${method} ${path}: not found`);
    return v;
  }

  // POST /v1/teams/resolve  { orgs: [{ orgId, name }] } -> { teams: Team[] }
  async resolveTeams(orgs: OrgRef[]): Promise<Team[]> {
    return (await this.must<{ teams: Team[] }>("POST", "/teams/resolve", { orgs })).teams;
  }

  // GET /v1/teams/:team/health -> TeamHealth
  getTeamHealth(team: string): Promise<TeamHealth> {
    return this.must("GET", `/teams/${id(team)}/health`);
  }

  // GET /v1/teams/:team/overview?range=7d -> Overview
  getOverview(team: string, range: Range): Promise<Overview> {
    return this.must("GET", `/teams/${id(team)}/overview?range=${range}`);
  }

  // GET /v1/teams/:team/repos -> { repos: RepoSummary[] }
  async listRepos(team: string): Promise<RepoSummary[]> {
    return (await this.must<{ repos: RepoSummary[] }>("GET", `/teams/${id(team)}/repos`)).repos;
  }

  // GET /v1/teams/:team/repos/:repo -> RepoDetail | 404
  getRepo(team: string, repo: string): Promise<RepoDetail | null> {
    return this.request("GET", `/teams/${id(team)}/repos/${id(repo)}`);
  }

  // GET /v1/teams/:team/runs?repo=&limit= -> { asOf, runs: RunSummary[] }  (newest first)
  listRuns(team: string, opts: { repo?: string; limit?: number } = {}): Promise<{ asOf: string; runs: RunSummary[] }> {
    const q = new URLSearchParams();
    if (opts.repo) q.set("repo", opts.repo);
    if (opts.limit) q.set("limit", String(opts.limit));
    return this.must("GET", `/teams/${id(team)}/runs?${q}`);
  }

  // GET /v1/teams/:team/repos/:repo/runs/:number -> RunDetail | 404
  getRun(team: string, repo: string, run: number): Promise<RunDetail | null> {
    return this.request("GET", `/teams/${id(team)}/repos/${id(repo)}/runs/${runSeg(run)}`);
  }

  // GET /v1/teams/:team/steps?repo= -> { steps: StepSummary[] }
  async listSteps(team: string, opts: { repo?: string } = {}): Promise<StepSummary[]> {
    const q = opts.repo ? `?repo=${seg(opts.repo)}` : "";
    return (await this.must<{ steps: StepSummary[] }>("GET", `/teams/${id(team)}/steps${q}`)).steps;
  }

  // GET /v1/teams/:team/repos/:repo/steps/:key -> StepTrust | 404
  getStepTrust(team: string, repo: string, key: string): Promise<StepTrust | null> {
    return this.request("GET", `/teams/${id(team)}/repos/${id(repo)}/steps/${id(key)}`);
  }

  // PUT /v1/teams/:team/repos/:repo/steps/:key/proofs  { need } -> StepTrust   (team admins)
  setProofsNeeded(team: string, repo: string, key: string, need: number): Promise<StepTrust> {
    return this.must("PUT", `/teams/${id(team)}/repos/${id(repo)}/steps/${id(key)}/proofs`, { need });
  }

  // POST /v1/teams/:team/repos/:repo/runs/:number/rerun  { quod: false } -> { accepted, message }
  requestRerunWithoutQuod(team: string, repo: string, run: number) {
    return this.must<{ accepted: boolean; message: string }>("POST", `/teams/${id(team)}/repos/${id(repo)}/runs/${runSeg(run)}/rerun`, {
      quod: false,
    });
  }

  // GET /v1/teams/:team/runners -> { runners: Runner[] }
  async listRunners(team: string): Promise<Runner[]> {
    return (await this.must<{ runners: Runner[] }>("GET", `/teams/${id(team)}/runners`)).runners;
  }

  // GET /v1/teams/:team/connect -> ConnectSetup
  getConnectSetup(team: string): Promise<ConnectSetup> {
    return this.must("GET", `/teams/${id(team)}/connect`);
  }

  // PUT /v1/teams/:team/connect  { store, host } -> 204
  async saveConnectChoice(team: string, choice: { store: Store; host: Host }): Promise<void> {
    await this.request("PUT", `/teams/${id(team)}/connect`, choice);
  }

  // GET /v1/teams/:team/search-index -> SearchIndex
  getSearchIndex(team: string): Promise<SearchIndex> {
    return this.must("GET", `/teams/${id(team)}/search-index`);
  }
}
