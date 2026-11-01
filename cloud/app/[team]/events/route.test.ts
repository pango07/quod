import { NextRequest } from "next/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

// The SSE route with the DAL and API mocked: who may open a team's stream.
const findTeam = vi.fn();
const subscribe = vi.fn();
const getActivity = vi.fn();

vi.mock("server-only", () => ({}));
vi.mock("next/server", async (orig) => ({ ...(await orig<typeof import("next/server")>()), connection: async () => {} }));
vi.mock("@/lib/dal", () => ({ findTeam: (...a: unknown[]) => findTeam(...a) }));
vi.mock("@/lib/api", () => ({ api: () => ({ subscribe: (...a: unknown[]) => subscribe(...a), getActivity: (...a: unknown[]) => getActivity(...a) }) }));

const { GET, HEAD } = await import("./route");

const call = (team: string, query = "", headers: Record<string, string> = {}) =>
  GET(new NextRequest(`http://localhost/${team}/events${query}`, { headers }), { params: Promise.resolve({ team }) } as never);

beforeEach(() => {
  findTeam.mockReset();
  subscribe.mockReset();
  getActivity.mockReset();
});

describe("GET /[team]/events", () => {
  it("is a 404 for a team the viewer is not a member of, and never opens a stream", async () => {
    findTeam.mockResolvedValue(null);
    const res = await call("acme");
    expect(res.status).toBe(404);
    expect(findTeam).toHaveBeenCalledWith("acme");
    expect(subscribe).not.toHaveBeenCalled();
    expect(getActivity).not.toHaveBeenCalled();
  });

  it("refuses a malformed scope", async () => {
    findTeam.mockResolvedValue({ team: { slug: "quodci" } });
    expect((await call("quodci", "?scope=run&repo=quod&run=abc")).status).toBe(400);
    expect((await call("quodci", "?scope=repo&repo=..")).status).toBe(400);
  });

  it("refuses a malformed Last-Event-ID", async () => {
    findTeam.mockResolvedValue({ team: { slug: "quodci" } });
    expect((await call("quodci", "", { "last-event-id": "has space" })).status).toBe(400);
    expect((await call("quodci", `?lastEventId=${"x".repeat(129)}`)).status).toBe(400);
    expect(subscribe).not.toHaveBeenCalled();
  });

  it("starts a new client's stream at its snapshot's cursor, so nothing in between is lost", async () => {
    findTeam.mockResolvedValue({ team: { slug: "quodci" } });
    getActivity.mockResolvedValue({ asOf: "x", jobs: [], simulated: true, cursor: "1000-999999999-9999" });
    subscribe.mockImplementation(async function* () {});
    const res = await call("quodci");
    expect(subscribe.mock.calls[0][2]).toMatchObject({ lastEventId: "1000-999999999-9999" });
    await res.body?.cancel();
  });

  it("answers the browser's probe without opening a stream", async () => {
    findTeam.mockResolvedValueOnce(null).mockResolvedValueOnce({ team: { slug: "quodci" } });
    expect((await HEAD(new NextRequest("http://localhost/acme/events", { method: "HEAD" }), { params: Promise.resolve({ team: "acme" }) } as never)).status).toBe(404);
    expect((await HEAD(new NextRequest("http://localhost/quodci/events", { method: "HEAD" }), { params: Promise.resolve({ team: "quodci" }) } as never)).status).toBe(200);
    expect(subscribe).not.toHaveBeenCalled();
  });

  it("streams to a member, uncached, resuming from Last-Event-ID", async () => {
    findTeam.mockResolvedValue({ team: { slug: "quodci" } });
    getActivity.mockResolvedValue({ asOf: "x", jobs: [], simulated: true, cursor: "9-9-9" });
    subscribe.mockImplementation(async function* () {});
    const res = await call("quodci", "?scope=repo&repo=quod", { "last-event-id": "1-2-3" });
    expect(res.status).toBe(200);
    expect(res.headers.get("content-type")).toBe("text/event-stream; charset=utf-8");
    expect(res.headers.get("cache-control")).toContain("no-store");
    expect(subscribe.mock.calls[0][0]).toBe("quodci");
    expect(subscribe.mock.calls[0][1]).toEqual({ kind: "repo", repo: "quod" });
    expect(subscribe.mock.calls[0][2]).toMatchObject({ lastEventId: "1-2-3" });
    await res.body?.cancel();
  });
});
