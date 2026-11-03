import Link from "next/link";
import { Dot, EmptyState } from "@/components/ui";
import type { BranchProtection, QuodStatus } from "@/lib/api/types";
import type { Section } from "@/lib/api/section";

// What a page shows when Quod's side cannot answer, and the writer-trust
// warning (DECISIONS.md 2026-11-02). Server components, data in props.

/** A section quod-server could not fill: "not available yet" (501) or "unavailable" (down). Never the whole page. */
export function SectionUnavailable({ s, what, page }: { s: Extract<Section<unknown>, { ok: false }>; what: string; page?: boolean }) {
  const card = (
    <section className="card" role="status">
      {s.kind === "not-available" ? (
        <EmptyState title={`${what}: not available yet`}>Quod&apos;s server does not provide this yet. It will appear here once it does.</EmptyState>
      ) : (
        <EmptyState title="Quod data unavailable">
          Quod&apos;s server did not answer, so {what.toLowerCase()} can&apos;t be shown right now. Your CI from GitHub is unaffected; reload in a minute.
        </EmptyState>
      )}
    </section>
  );
  // In place of a whole page: with the page's own padding.
  return page ? <div className="page">{card}</div> : card;
}

/** A one-line note when a page shows GitHub's data alone because quod-server did not answer. */
export function QuodNote({ status }: { status: QuodStatus | undefined }) {
  if (status?.state !== "unavailable") return null;
  return (
    <p className="notice" role="status" style={{ alignItems: "center" }}>
      <Dot tone="amber" />
      <span>
        <span style={{ fontWeight: 500 }}>Quod data unavailable.</span> <span className="soft">Showing your CI from GitHub; Quod&apos;s numbers come back when its server answers.</span>
      </span>
    </p>
  );
}

const LEARN_MORE = (team: string) => `/${team}/trust#protected-branch`;

const STATE_LABEL: Record<BranchProtection["state"], string> = {
  protected: "protected",
  unprotected: "not protected",
  unknown: "protection unknown",
};

/** A repo's default-branch protection, as a small chip. */
export function ProtectionBadge({ p }: { p: BranchProtection | undefined }) {
  if (!p) return null;
  const branch = p.branch ?? "Default branch";
  const tone = p.state === "protected" ? "green" : p.state === "unprotected" ? "amber" : "";
  const title =
    p.state === "protected"
      ? `${branch} is protected on GitHub.`
      : p.state === "unprotected"
        ? `${branch} is not protected: anyone who can push to it can record results every CI job will reuse.`
        : p.detail === "plan"
          ? "GitHub offers branch protection for this private repository only on a paid plan, so Quod can't check it."
          : protectionLabel(p) === "checking protection"
            ? "Quod is reading this branch's protection from GitHub."
            : "Quod could not read this branch's protection (the GitHub App needs Administration: read, or GitHub did not answer).";
  return (
    <span className={`chip ${tone}`} title={title}>
      <Dot tone={p.state === "protected" ? "green" : p.state === "unprotected" ? "amber" : "dim"} size="sm" />
      {protectionLabel(p)}
    </span>
  );
}

/**
 * The badge's words. "checking protection" only while a first read is under
 * way (`pending`, a few seconds at most): never a spinner that stays up.
 */
export function protectionLabel(p: BranchProtection): string {
  const branch = p.branch ?? "Default branch";
  if (p.state !== "unknown") return `${branch} ${STATE_LABEL[p.state]}`;
  if (p.detail === "plan") return "Protection: can't check on this plan";
  if (p.pending && !p.checkedAt) return "checking protection";
  return "Protection: unknown";
}

/**
 * The writer-trust warning for one repo, or for the team (`repos` > 1). Shown
 * while trusted writes may come from a default branch anyone with push access
 * can write to; "unknown" says protection could not be read.
 */
export function ProtectionWarning({ team, state, repos, branch }: { team: string; state: "unprotected" | "unknown"; repos: { slug: string; fullName: string }[]; branch?: string | null }) {
  const b = branch ?? (repos.length === 1 ? "main" : "the default branch");
  const names = repos.slice(0, 3).map((r, i) => (
    <span key={r.slug}>
      {i ? ", " : ""}
      <Link href={`/${team}/${r.slug}`} className="mono">
        {r.fullName}
      </Link>
    </span>
  ));
  return (
    <div className="notice" role="note" aria-label="Branch protection" style={{ alignItems: "flex-start" }}>
      <Dot tone="amber" />
      <span style={{ display: "flex", flexDirection: "column", gap: 4 }}>
        <span style={{ fontWeight: 500 }}>
          {state === "unprotected" ? (
            <>
              {repos.length === 1 ? `${b} is not protected` : `${repos.length} repositories have an unprotected default branch`}
            </>
          ) : (
            <>Branch protection unknown</>
          )}
        </span>
        <span className="soft">
          {state === "unprotected" ? (
            <>
              Anyone who can push to {b} can record results that every CI job will reuse; protect {b} or turn on require_protected_ref.
            </>
          ) : (
            <>Quod can&apos;t read whether {b} is protected yet (the GitHub App needs Administration: read). If it is not, anyone who can push to it can record results every CI job will reuse.</>
          )}{" "}
          {repos.length > 1 ? <>{names}{repos.length > 3 ? ` and ${repos.length - 3} more` : ""}. </> : null}
          <Link href={LEARN_MORE(team)}>Learn more</Link>
        </span>
      </span>
    </div>
  );
}

/** Which repos the warning is about: those Quod writes trusted results for (it has data) whose default branch is not, or not known to be, protected. */
export function protectionConcern(repos: { slug: string; fullName: string; hasQuodData?: boolean; protection?: BranchProtection }[]) {
  const quod = repos.filter((r) => r.hasQuodData !== false && r.protection);
  const unprotected = quod.filter((r) => r.protection!.state === "unprotected");
  if (unprotected.length) return { state: "unprotected" as const, repos: unprotected };
  // Not read yet (checkedAt null) is not "the App cannot read it": no warning until a read says so.
  const unknown = quod.filter((r) => r.protection!.state === "unknown" && r.protection!.checkedAt !== null);
  if (unknown.length) return { state: "unknown" as const, repos: unknown };
  return null;
}
