// Runs once when a server starts. In production it refuses to start without
// a complete WorkOS config and a data source (fail closed). `next build`
// is exempt: it serves no requests, so it can run without secrets.

export async function register() {
  if (process.env.NEXT_RUNTIME !== "nodejs") return;
  if (process.env.NEXT_PHASE === "phase-production-build") return;
  if (process.env.NODE_ENV !== "production") return;

  const { authMode } = await import("./lib/auth/config");
  const { resolveApiMode } = await import("./lib/api/mode");
  authMode(); // throws AuthConfigError
  const data = resolveApiMode(process.env);
  if (data.kind === "misconfigured") throw new Error(`Quod Cloud data source is not configured. ${data.problem}`);
}
