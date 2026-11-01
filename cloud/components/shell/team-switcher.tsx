"use client";

import Link from "next/link";
import { useTransition } from "react";
import { Icon } from "@/components/ui";
import { switchTeam } from "@/lib/actions";
import { Popover } from "./popover";

interface TeamItem {
  slug: string;
  name: string;
  plan: string;
  role: string;
}

const Mark = () => <span style={{ width: 18, height: 18, borderRadius: "50%", background: "linear-gradient(135deg,#fff,#666)", flex: "none" }} aria-hidden="true" />;

export function TeamSwitcher({ current, teams }: { current: string; teams: TeamItem[] }) {
  const team = teams.find((t) => t.slug === current) ?? teams[0];
  const [pending, start] = useTransition();
  return (
    <Popover
      label={`Team: ${team.name}. Switch team`}
      buttonClass="team-btn"
      button={
        <>
          <Mark />
          <span className="ellipsis team-name" style={{ maxWidth: 160 }}>
            {team.name}
          </span>
          <span className="chip small">{team.plan}</span>
          <Icon name="updown" size={14} stroke="var(--faint)" />
        </>
      }
    >
      {(close) => (
        <>
          <div className="menu-label">Teams</div>
          {teams.map((t) => (
            <button
              key={t.slug}
              type="button"
              role="menuitemradio"
              aria-checked={t.slug === current}
              className="menu-item"
              disabled={pending}
              onClick={() => {
                close();
                if (t.slug !== current) start(() => switchTeam(t.slug));
              }}
            >
              <Mark />
              <span style={{ flex: 1, minWidth: 0 }} className="ellipsis">
                {t.name}
              </span>
              <span style={{ fontSize: 12, color: "var(--faint)" }}>{t.role === "admin" ? "Admin" : "Member"}</span>
              <span style={{ width: 16, display: "inline-flex" }}>{t.slug === current ? <Icon name="check" size={14} /> : null}</span>
            </button>
          ))}
          <div className="menu-sep" />
          <Link role="menuitem" className="menu-item" href={`/${current}/settings`} onClick={close}>
            Team settings
          </Link>
          <Link role="menuitem" className="menu-item" href={`/${current}/connect`} onClick={close}>
            <Icon name="plus" size={14} /> Connect GitHub
          </Link>
        </>
      )}
    </Popover>
  );
}
