// No Quod server (QUOD_API unset, or quod-server has no credentials for this
// team): Quod's side of the CloudApi is empty. Every page still renders the
// team's GitHub CI through the composite, and Quod's numbers say "Appears
// once Quod runs". Never sample data.

import { CONNECT, EMPTY_TOTALS } from "./fixture-data";
import { resolveTeamsLocally } from "./teams";
import { ApiNotAvailableError, type Activity, type CloudApi, type ConnectSetup, type LiveEvent, type Overview, type Range, type TestRunDetail, type TestRunLite, type TestRunSummary, type TestStats, type TestSuite } from "./types";

const now = () => new Date().toISOString();

const sleep = (ms: number, signal: AbortSignal) =>
  new Promise<void>((resolve) => {
    const t = setTimeout(resolve, ms);
    signal.addEventListener("abort", () => (clearTimeout(t), resolve()), { once: true });
  });

const off = (what: string) => new ApiNotAvailableError("Not available yet", `${what} needs a Quod server; this deploy has none yet.`);

export class NoQuodCloudApi implements CloudApi {
  async resolveTeams(orgs: Parameters<CloudApi["resolveTeams"]>[0]) {
    return resolveTeamsLocally(orgs);
  }

  async getTeamHealth() {
    return { wrongReplays: 0, spotChecks: 0, pausedSteps: 0, repos: 0 };
  }

  async getOverview(_team: string, range: Range): Promise<Overview> {
    return { asOf: now(), totals: EMPTY_TOTALS(range), days: [], repos: [], trust: { proven: 0, proving: 0, watching: 0, spotChecksPassed: 0, paused: [] }, runners: [], quod: { state: "off" } };
  }

  async listRepos() {
    return [];
  }

  async getRepo() {
    return null;
  }

  async listRuns() {
    return { asOf: now(), runs: [], quod: { state: "off" as const } };
  }

  async getRun() {
    return null;
  }

  async listSteps() {
    return [];
  }

  async getStepTrust() {
    return null;
  }

  async setProofsNeeded(): Promise<never> {
    throw off("Changing proofs");
  }

  async requestRerunWithoutQuod(): Promise<never> {
    throw off("Re-running without Quod");
  }

  async listRunners() {
    return [];
  }

  async getConnectSetup(): Promise<ConnectSetup> {
    return { ...CONNECT, runnerLabels: [...CONNECT.runnerLabels], firstRepo: null, saved: null };
  }

  async saveConnectChoice(): Promise<never> {
    throw off("Saving the setup choice");
  }

  async getSearchIndex() {
    return { repos: [], runs: [], steps: [] };
  }

  async getActivity(): Promise<Activity> {
    return { asOf: now(), jobs: [], cursor: null, simulated: false, quod: { state: "off" } };
  }

  /** No server, no test runs: the test sections are hidden. */
  async listTestRuns(): Promise<TestRunSummary[] | null> {
    return null;
  }

  async getTestRun(): Promise<TestRunDetail | null> {
    return null;
  }

  async getTestStats(): Promise<TestStats | null> {
    return null;
  }

  async listTestSuites(): Promise<TestSuite[] | null> {
    return null;
  }

  async listTestHistory(): Promise<TestRunLite[] | null> {
    return null;
  }

  /** Nothing ever happens: hold the stream open quietly (the composite merges GitHub's events next to it). */
  async *subscribe(_team: string, _scope: unknown, opts: { signal: AbortSignal }): AsyncIterable<LiveEvent> {
    while (!opts.signal.aborted) await sleep(5000, opts.signal);
  }
}
