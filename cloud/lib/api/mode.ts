// Which CloudApi adapter serves this process. Pure, so it can be tested.
//
// QUOD_API_URL set: quod-server over HTTP. QUOD_API=fixture: the fixture
// adapter (sample data), only ever by that explicit flag (cloud/.env.development
// sets it for `next dev`). Neither: misconfigured, so no deploy shows sample
// data by accident.

export type ApiMode = { kind: "http"; url: string } | { kind: "fixture" } | { kind: "misconfigured"; problem: string };

export function resolveApiMode(env: Partial<Record<string, string | undefined>>): ApiMode {
  const url = env.QUOD_API_URL?.trim();
  if (url) {
    try {
      new URL(url);
      return { kind: "http", url };
    } catch {
      return { kind: "misconfigured", problem: "QUOD_API_URL is not a URL." };
    }
  }
  if (env.QUOD_API === "fixture") return { kind: "fixture" };
  return { kind: "misconfigured", problem: "Set QUOD_API_URL (quod-server), or QUOD_API=fixture to serve sample data on purpose." };
}
