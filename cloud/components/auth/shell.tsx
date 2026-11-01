import type { ReactNode } from "react";
import { Logo, Sk } from "@/components/ui";

/** The sign-in pages: Quod logo, a heading, one centered card, links under it. Phone first. */
export function AuthShell({ title, lede, children, footer, aside }: { title: string; lede?: ReactNode; children: ReactNode; footer?: ReactNode; aside?: ReactNode }) {
  return (
    <main id="main" className="auth-page">
      <div className="auth-head">
        <Logo size={32} />
        <h1>{title}</h1>
        {lede ? <p className="muted">{lede}</p> : null}
      </div>
      <div className="card auth-card">{children}</div>
      {footer ? <div className="auth-foot">{footer}</div> : null}
      {aside}
    </main>
  );
}

export function AuthSkeleton() {
  return (
    <main className="auth-page" aria-busy="true">
      <div className="auth-head">
        <Logo size={32} />
        <Sk w={200} h={26} />
      </div>
      <div className="card auth-card">
        <Sk h={14} w={80} />
        <Sk h={40} r={8} />
        <Sk h={40} r={8} />
      </div>
      <span className="sr-only">Loading</span>
    </main>
  );
}

/** Shown in the local demo session, where the forms are previews and nothing is sent. */
export function DemoNote({ previews }: { previews?: { href: string; label: string }[] }) {
  return (
    <aside className="auth-demo" aria-label="Demo session">
      <p>
        <span className="demo-chip">Demo session</span> WorkOS is not configured, so these forms are previews and nothing is sent.
      </p>
      {previews?.length ? (
        <p className="auth-previews">
          Preview:{" "}
          {previews.map((p, i) => (
            <span key={p.href}>
              {i ? " · " : ""}
              <a href={p.href}>{p.label}</a>
            </span>
          ))}
        </p>
      ) : null}
    </aside>
  );
}
