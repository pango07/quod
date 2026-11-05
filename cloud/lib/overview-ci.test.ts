import { afterEach, describe, expect, it, vi } from "vitest";
vi.mock("server-only", () => ({}));
vi.mock("next/navigation", () => ({unstable_rethrow:() => undefined}));
const mocks = vi.hoisted(() => ({github:vi.fn(), runs:vi.fn(), sampleRuns:vi.fn()}));
vi.mock("./github/team-data", () => ({teamGitHub:mocks.github}));
vi.mock("./github/pages", () => ({listTeamRuns:mocks.runs}));
vi.mock("./api", () => ({api:() => ({listRuns:mocks.sampleRuns})}));
import { overviewCi } from "./overview-ci";

afterEach(() => { vi.useRealTimers(); vi.resetAllMocks(); });
describe("overview workflow duration", () => {
  it("uses scoped GitHub elapsed times and excludes incomplete, old and invalid observations", async () => {
    vi.useFakeTimers(); vi.setSystemTime(new Date("2026-11-05T12:00:00Z"));
    mocks.github.mockResolvedValue({sample:false,source:{teamId:"scoped-team",db:"db"}});
    const row = {status:"completed", startedAt:"2026-11-04T12:00:00Z", outcome:"passed", repoFullName:"team/app", title:"CI", htmlUrl:"https://github.com/team/app/actions/runs/1"};
    mocks.runs.mockResolvedValue({runs:[
      {...row,id:1,durationSeconds:20}, {...row,id:2,durationSeconds:40},
      {...row,id:3,durationSeconds:null}, {...row,id:4,status:"in_progress",durationSeconds:200},
      {...row,id:5,startedAt:"2026-10-01T00:00:00Z",durationSeconds:900},
    ]});
    const result = await overviewCi({slug:"team",orgId:"org"}, "7d");
    expect(mocks.runs).toHaveBeenCalledWith("scoped-team","db",{perPage:100});
    expect(result).toMatchObject({medianSeconds:30,measuredRuns:2,sample:false});
    expect(mocks.sampleRuns).not.toHaveBeenCalled();
  });
  it("does not replace missing GitHub duration with summed Quod steps", async () => {
    mocks.github.mockResolvedValue({sample:false,source:null});
    const result = await overviewCi({slug:"team",orgId:"org"}, "7d");
    expect(result.medianSeconds).toBeNull();
    expect(mocks.sampleRuns).not.toHaveBeenCalled();
  });
  it("labels demo durations as sample data", async () => {
    vi.useFakeTimers(); vi.setSystemTime(new Date("2026-11-05T12:00:00Z"));
    mocks.github.mockResolvedValue({sample:true,source:null});
    mocks.sampleRuns.mockResolvedValue({runs:[{status:"passed",startedAt:"2026-11-04T12:00:00Z",repoSlug:"app",number:1,title:"Demo",tookSeconds:25}]});
    expect(await overviewCi({slug:"demo",orgId:"sample"}, "7d")).toMatchObject({medianSeconds:25,measuredRuns:1,sample:true});
    expect(mocks.runs).not.toHaveBeenCalled();
  });
});
