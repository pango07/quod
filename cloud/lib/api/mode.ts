// Which CloudApi adapter serves this process. Pure, so it can be tested.
//
// QUOD_API_URL set: quod-server over HTTP. Otherwise the fixture adapter
// (sample data), but in production only when QUOD_API=fixture says so
// explicitly: a production deploy never shows sample data by accident.

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
  if (env.QUOD_API === "fixture" || env.NODE_ENV !== "production") return { kind: "fixture" };
  return { kind: "misconfigured", problem: "Set QUOD_API_URL (quod-server), or QUOD_API=fixture to serve sample data on purpose." };
}
