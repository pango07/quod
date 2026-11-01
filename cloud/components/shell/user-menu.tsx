"use client";

import Link from "next/link";
import { Avatar } from "@/components/ui";
import { signOutAction } from "@/lib/actions";
import { Popover } from "./popover";

export function UserMenu({ name, email, avatarUrl, team }: { name: string; email: string; avatarUrl: string | null; team: string }) {
  return (
    <Popover label={`Account: ${name}`} buttonClass="avatar" align="right" button={<Avatar name={name} url={avatarUrl} />}>
      {(close) => (
        <>
          <div style={{ padding: "8px 10px 10px", display: "flex", flexDirection: "column", gap: 2 }}>
            <span style={{ fontWeight: 500 }}>{name}</span>
            <span style={{ fontSize: 13, color: "var(--faint)" }}>{email}</span>
          </div>
          <div className="menu-sep" />
          <Link role="menuitem" className="menu-item" href={`/${team}/settings`} onClick={close}>
            Team settings
          </Link>
          <a role="menuitem" className="menu-item" href="https://www.quodci.com/under-the-hood">
            How Quod works
          </a>
          <div className="menu-sep" />
          <form action={signOutAction}>
            <button role="menuitem" type="submit" className="menu-item">
              Sign out
            </button>
          </form>
        </>
      )}
    </Popover>
  );
}
