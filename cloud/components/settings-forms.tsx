"use client";

import { useActionState, useState, useTransition } from "react";
import { inviteMember, openAdminPortal, saveRunnerRate, type ActionResult } from "@/lib/actions";
import type { PortalIntent } from "@/lib/auth/directory";

export function InviteForm({ team }: { team: string }) {
  const [state, action, pending] = useActionState(inviteMember.bind(null, team), null as ActionResult);
  return (
    <form action={action} style={{ display: "flex", flexDirection: "column", gap: 10 }}>
      <div style={{ display: "flex", gap: 8, flexWrap: "wrap", alignItems: "flex-end" }}>
        <label className="field" style={{ flex: "1 1 240px" }}>
          Email
          <input className="input" name="email" type="email" required placeholder="dev@example.com" autoComplete="off" />
        </label>
        <label className="field">
          Role
          <select className="select" name="role" defaultValue="member">
            <option value="member">Member</option>
            <option value="admin">Admin</option>
          </select>
        </label>
        <button className="btn solid" type="submit" disabled={pending} style={{ height: 36 }}>
          {pending ? "Sending…" : "Invite"}
        </button>
      </div>
      {state ? (
        <p role="status" className={`toast ${state.ok ? "ok" : "err"}`}>
          {state.message}
        </p>
      ) : null}
    </form>
  );
}

const PORTAL: { intent: PortalIntent; title: string; body: string; cta: string }[] = [
  { intent: "sso", title: "Single sign-on (SAML or OIDC)", body: "Sign in through Okta, Microsoft Entra, Google Workspace or any SAML or OIDC provider.", cta: "Set up SSO" },
  { intent: "dsync", title: "Directory sync (SCIM)", body: "Add and remove team members from your identity provider. People who leave the company lose access.", cta: "Set up directory sync" },
  { intent: "domain_verification", title: "Verified domain", body: "Prove you own your email domain, so people from it sign in through your SSO.", cta: "Verify a domain" },
];

/**
 * SSO & directory sync. Each button asks the server for a short-lived WorkOS
 * Admin Portal link and opens it; the portal walks your IT admin through the setup.
 */
export function PortalButtons({ team, canEdit }: { team: string; canEdit: boolean }) {
  const [pending, start] = useTransition();
  const [busy, setBusy] = useState<PortalIntent | null>(null);
  const [result, setResult] = useState<ActionResult>(null);
  return (
    <div style={{ display: "flex", flexDirection: "column" }}>
      {PORTAL.map((p, i) => (
        <div key={p.intent} className="row-between" style={{ padding: "14px 0", borderTop: i ? "1px solid var(--rule)" : 0 }}>
          <div style={{ display: "flex", flexDirection: "column", gap: 2, flex: "1 1 320px", minWidth: 0 }}>
            <span style={{ fontWeight: 500 }}>{p.title}</span>
            <span className="muted" style={{ fontSize: 13 }}>
              {p.body}
            </span>
          </div>
          <button
            type="button"
            className="btn"
            disabled={!canEdit || pending}
            title={canEdit ? undefined : "Only team admins can change this"}
            onClick={() => {
              setBusy(p.intent);
              setResult(null);
              start(async () => {
                setResult(await openAdminPortal(team, p.intent));
                setBusy(null);
              });
            }}
          >
            {pending && busy === p.intent ? "Opening…" : p.cta}
          </button>
        </div>
      ))}
      {result ? (
        <p role="status" className={`toast ${result.ok ? "ok" : "err"}`} style={{ marginTop: 8 }}>
          {result.message}
        </p>
      ) : null}
    </div>
  );
}

/** The runner price the Overview's money saved is estimated with. Empty: GitHub's Linux 2-core price. */
export function RunnerRateForm({ team, current, custom, canEdit, canSave, defaultRate }: { team: string; current: number; custom: boolean; canEdit: boolean; canSave: boolean; defaultRate: number }) {
  const [state, action, pending] = useActionState(saveRunnerRate.bind(null, team), null as ActionResult);
  const disabled = !canEdit || !canSave;
  return (
    <form action={action} style={{ display: "flex", flexDirection: "column", gap: 10 }}>
      <div style={{ display: "flex", gap: 8, flexWrap: "wrap", alignItems: "flex-end" }}>
        <label className="field" style={{ flex: "0 1 220px" }}>
          Price per runner minute (USD)
          <input
            className="input mono"
            name="rate"
            inputMode="decimal"
            placeholder={String(defaultRate)}
            defaultValue={custom ? String(current) : ""}
            disabled={disabled}
            aria-describedby="rate-help"
            autoComplete="off"
          />
        </label>
        <button className="btn solid" type="submit" disabled={disabled || pending} style={{ height: 36 }} title={canEdit ? undefined : "Only team admins can change this"}>
          {pending ? "Saving…" : "Save price"}
        </button>
      </div>
      <p id="rate-help" className="faint" style={{ fontSize: 12, lineHeight: 1.5, maxWidth: "70ch" }}>
        Leave it empty to use ${defaultRate} per minute, GitHub&apos;s published price for a Linux 2-core runner. Set your own if you use larger runners, self-hosted machines or have a
        negotiated price.
        {canSave ? null : " Saving a price isn't set up on this deploy yet, so the default is used."}
      </p>
      {state ? (
        <p role="status" className={`toast ${state.ok ? "ok" : "err"}`}>
          {state.message}
        </p>
      ) : null}
    </form>
  );
}
