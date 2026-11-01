"use client";

import { useState } from "react";
import { saveConnectChoice } from "@/lib/actions";
import type { ConnectSetup, Host, Store } from "@/lib/api/types";

const STORES: { id: Store; title: string; body: string }[] = [
  { id: "cloud", title: "Quod Cloud", body: "We run it. Encrypted with your team’s key; source code never leaves your runner." },
  { id: "self", title: "Self-hosted", body: "Run quod-server yourself: one binary, SQLite or Postgres, any S3 bucket. Open source." },
];
const HOSTS: { id: Host; title: string; body: string }[] = [
  { id: "github", title: "GitHub-hosted runners", body: "Add one step. Nothing to install or manage." },
  { id: "own", title: "Your own runners", body: "Point jobs at runners with Quod installed. Fastest replays." },
];

function workflow(setup: ConnectSetup, store: Store, host: Host) {
  const endpoint = store === "cloud" ? "" : `\n        with:\n          server: ${setup.selfHostedExample}`;
  return host === "github"
    ? {
        title: "Add one step",
        before: "jobs:\n  test:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@v4\n",
        minus: "",
        plus: `      - uses: ${setup.setupAction}${endpoint}`,
        after: "      - run: go test ./...",
        note: store === "cloud" ? "Signs in with GitHub OIDC. No secrets to copy." : "Signs in to your server with GitHub OIDC.",
      }
    : {
        title: "Change one line",
        before: "jobs:\n  test:\n",
        minus: "    runs-on: ubuntu-latest",
        plus: `    runs-on: [${setup.runnerLabels.join(", ")}]`,
        after: "    steps:\n      - uses: actions/checkout@v4\n      - run: go test ./...",
        note: `Install the runner with one command: ${setup.installCommand}`,
      };
}

function Options<T extends string>({ label, options, value, onChange }: { label: string; options: { id: T; title: string; body: string }[]; value: T; onChange: (v: T) => void }) {
  return (
    <div role="group" aria-label={label} style={{ display: "flex", gap: 12, flexWrap: "wrap" }}>
      {options.map((o) => (
        <button key={o.id} type="button" className="opt" aria-pressed={o.id === value} onClick={() => onChange(o.id)}>
          <span style={{ fontWeight: 600 }}>{o.title}</span>
          <span style={{ color: "var(--muted)", fontSize: 13, lineHeight: 1.5 }}>{o.body}</span>
        </button>
      ))}
    </div>
  );
}

/** Where to join early access: the landing page's waitlist (web/app/page.tsx, section id "join"). */
export const EARLY_ACCESS_URL = "https://www.quodci.com/#join";

/**
 * Connect.dc.html's Quod setup: two choices, the workflow change they imply,
 * and what happens next. Not released yet (the setup action and the runner
 * install do not exist), so it is shown as "Coming soon: early access":
 * dimmed, the workflow change not selectable, and a link to join.
 */
export function ConnectFlow({ team, setup, initial, canSave }: { team: string; setup: ConnectSetup; initial: { store: Store; host: Host }; canSave: boolean }) {
  const [store, setStore] = useState(initial.store);
  const [host, setHost] = useState(initial.host);
  const change = workflow(setup, store, host);

  const update = (s: Store, h: Host) => {
    setStore(s);
    setHost(h);
    const url = new URL(window.location.href);
    url.searchParams.set("store", s);
    url.searchParams.set("host", h);
    window.history.replaceState(null, "", url);
    // Only admins save the team's choice; members can still compare both setups.
    if (canSave) void saveConnectChoice(team, s, h);
  };

  return (
    <section className="soon" aria-labelledby="soon-h" aria-describedby="soon-note">
      <div className="soon-head">
        <h2 id="soon-h" className="h2" style={{ fontSize: 17 }}>
          Run Quod in your CI
        </h2>
        <span className="chip amber">Coming soon: early access</span>
      </div>
      <p id="soon-note" className="soon-banner" role="note">
        This part doesn&apos;t work yet: the setup step and the runner install below aren&apos;t released. They show how it will work.{" "}
        <a href={EARLY_ACCESS_URL}>Join early access</a> to be among the first teams to run it.
      </p>
      <div className="soon-body">
      {canSave ? null : (
        <p className="toast" role="note">
          You can compare both setups. Only team admins can save the team’s choice.
        </p>
      )}
      <section aria-labelledby="where" style={{ display: "flex", flexDirection: "column", gap: 12 }}>
        <h2 id="where" className="h2">
          2. Where should Quod keep results?
        </h2>
        <Options label="Where Quod keeps results" options={STORES} value={store} onChange={(s) => update(s, host)} />
      </section>

      <section aria-labelledby="runs" style={{ display: "flex", flexDirection: "column", gap: 12 }}>
        <h2 id="runs" className="h2">
          3. Where do your jobs run?
        </h2>
        <Options label="Where your jobs run" options={HOSTS} value={host} onChange={(h) => update(store, h)} />
      </section>

      <section aria-labelledby="change" className="card" style={{ overflow: "hidden" }}>
        <div className="card-head ruled" style={{ padding: "14px 18px" }}>
          <h2 id="change" className="h2" aria-live="polite">
            4. {change.title} <span className="faint" style={{ fontWeight: 400, fontSize: 13 }}>(preview)</span>
          </h2>
          <span className="mono faint" style={{ fontSize: 12 }}>
            .github/workflows/ci.yml
          </span>
        </div>
        {/* Not selectable: copying this into a workflow would break it until the action is released. */}
        <pre className="wf no-copy" aria-label="Workflow change (preview; not released yet)" onCopy={(e) => e.preventDefault()}>
          {change.before}
          {change.minus ? (
            <span className="minus">
              <span className="sr-only">Remove: </span>
              {change.minus}
            </span>
          ) : null}
          <span className="plus">
            <span className="sr-only">Add: </span>
            {change.plus}
          </span>
          {change.after}
        </pre>
        <div style={{ padding: "12px 18px", borderTop: "1px solid var(--rule)", fontSize: 13, color: "var(--muted)", overflowWrap: "anywhere" }}>{change.note}</div>
      </section>

      <section aria-labelledby="next" style={{ display: "flex", flexDirection: "column", gap: 12 }}>
        <h2 id="next" className="h2">
          5. Push, and watch it learn
        </h2>
        <ol style={{ margin: 0, padding: 0, listStyle: "none", display: "flex", flexDirection: "column", gap: 10 }}>
          {[
            ["First push:", "everything runs as usual. Quod only watches and notes what each step reads."],
            ["Second push:", "a step that gives the same result from the same inputs is proven."],
            ["From then on:", "if nothing it reads changed, you get its result in under a second. Quod re-checks a few at random."],
          ].map(([h, b], i) => (
            <li key={h} style={{ display: "flex", gap: 12, alignItems: "flex-start", fontSize: 14 }}>
              <span className="num" aria-hidden="true">
                {i + 1}
              </span>
              <span>
                <span style={{ fontWeight: 500 }}>{h}</span> <span className="muted">{b}</span>
              </span>
            </li>
          ))}
        </ol>
      </section>

      </div>
      <div style={{ display: "flex", gap: 10, justifyContent: "flex-end", flexWrap: "wrap" }}>
        <a className="btn lg" href="https://www.quodci.com/under-the-hood">
          How it works
        </a>
        <a className="btn solid lg" href={EARLY_ACCESS_URL}>
          Join early access
        </a>
      </div>
    </section>
  );
}
