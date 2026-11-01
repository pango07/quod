"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { Icon } from "@/components/ui";
import { NAV, TEAM_PAGES } from "@/lib/routes";

/** Which sidebar item a path belongs to: repo pages under Repositories, a run under Runs, a step under Trust. */
function section(pathname: string): string {
  const rest = pathname.split("/").filter(Boolean).slice(1);
  if (rest[0] === undefined) return "";
  if (TEAM_PAGES.includes(rest[0])) return rest[0] === "connect" ? "/runners" : `/${rest[0]}`;
  if (rest[1] === "runs" && rest[2]) return "/runs";
  if (rest[1] === "steps" && rest[2]) return "/trust";
  return "/repositories";
}

export function NavLinks({ team }: { team: string }) {
  const current = section(usePathname());
  return (
    <>
      {NAV.map((item) => (
        <Link key={item.label} className="nav-item" href={`/${team}${item.href}`} aria-current={current === item.href ? "page" : undefined}>
          <Icon name={item.icon} />
          {item.label}
        </Link>
      ))}
    </>
  );
}
