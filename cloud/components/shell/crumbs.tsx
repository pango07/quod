"use client";

import Link from "next/link";
import { useSelectedLayoutSegments } from "next/navigation";
import { TEAM_PAGES } from "@/lib/routes";

/** The rest of the breadcrumb after the team: repo, then run or step. */
export function Crumbs({ team }: { team: string }) {
  const segs = useSelectedLayoutSegments().filter((s) => !s.startsWith("("));
  const [first, kind, id] = segs;
  if (!first || TEAM_PAGES.includes(first)) return null;
  const repo = decodeURIComponent(first);
  const leaf = kind === "runs" && id ? `run #${id}` : kind === "steps" && id ? "Trust" : null;
  return (
    <>
      <span className="sep" aria-hidden="true">
        /
      </span>
      {leaf ? (
        <Link href={`/${team}/${first}`} style={{ fontWeight: 500 }} className="ellipsis repo-crumb">
          {repo}
        </Link>
      ) : (
        <span style={{ fontWeight: 500 }} className="ellipsis repo-crumb" aria-current="page">
          {repo}
        </span>
      )}
      {leaf ? (
        <>
          <span className="sep leaf-crumb" aria-hidden="true">
            /
          </span>
          <span className={`last leaf-crumb ${kind === "runs" ? "mono" : ""}`} aria-current="page">
            {leaf}
          </span>
        </>
      ) : null}
    </>
  );
}
