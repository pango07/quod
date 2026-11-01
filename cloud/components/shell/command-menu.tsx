"use client";

import { useRouter } from "next/navigation";
import { useEffect, useId, useMemo, useRef, useState } from "react";
import { Dot, Icon } from "@/components/ui";
import type { SearchIndex } from "@/lib/api/types";
import { NAV } from "@/lib/routes";
import { trustTone } from "@/lib/trust";

interface Item {
  id: string;
  group: "Pages" | "Repositories" | "Runs" | "Steps";
  label: string;
  hint: string;
  href: string;
  tone?: "blue" | "amber" | "green" | "gray" | "dim" | "red";
}

function build(team: string, index: SearchIndex): Item[] {
  return [
    ...NAV.map((n) => ({ id: `p${n.href}`, group: "Pages" as const, label: n.label, hint: "", href: `/${team}${n.href}` })),
    { id: "p/connect", group: "Pages", label: "Connect a repository", hint: "", href: `/${team}/connect` },
    ...index.repos.map((r) => ({ id: `r${r.slug}`, group: "Repositories" as const, label: r.fullName, hint: "", href: `/${team}/${r.slug}` })),
    ...index.runs.map((r) => ({
      id: `u${r.repoSlug}${r.number}`,
      group: "Runs" as const,
      label: `#${r.number} ${r.title}`,
      hint: r.repoSlug,
      href: `/${team}/${r.repoSlug}/runs/${r.number}`,
      tone: (r.status === "failed" ? "red" : r.status === "running" ? "blue" : "green") as Item["tone"],
    })),
    ...index.steps.map((s) => ({
      id: `s${s.key}`,
      group: "Steps" as const,
      label: s.command,
      hint: `${s.repoSlug} · ${s.trust}`,
      href: `/${team}/${s.repoSlug}/steps/${s.key}`,
      tone: trustTone(s.trust),
    })),
  ];
}

const typing = (el: EventTarget | null) =>
  el instanceof HTMLElement && (el.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(el.tagName));

/** ⌘K (or Ctrl+K, or /) opens a menu to jump to any page, repository, run or step. */
export function CommandMenu({ team, index }: { team: string; index: SearchIndex }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const router = useRouter();
  const [q, setQ] = useState("");
  const [active, setActive] = useState(0);
  const [mac, setMac] = useState(true);
  const listId = useId();
  const all = useMemo(() => build(team, index), [team, index]);

  const items = useMemo(() => {
    const words = q.toLowerCase().split(/\s+/).filter(Boolean);
    const hits = words.length ? all.filter((i) => words.every((w) => `${i.label} ${i.hint} ${i.group}`.toLowerCase().includes(w))) : all;
    return hits.slice(0, 40);
  }, [all, q]);

  const open = () => {
    setQ("");
    setActive(0);
    dialog.current?.showModal();
    input.current?.focus();
  };

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- the platform is only known in the browser
    setMac(/Mac|iPhone|iPad/.test(navigator.platform));
    const onKey = (e: KeyboardEvent) => {
      if ((e.key === "k" || e.key === "K") && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        if (dialog.current?.open) dialog.current.close();
        else open();
      } else if (e.key === "/" && !typing(e.target) && !dialog.current?.open) {
        e.preventDefault();
        open();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  useEffect(() => {
    document.getElementById(`${listId}-${active}`)?.scrollIntoView({ block: "nearest" });
  }, [active, listId]);

  const go = (item: Item | undefined) => {
    if (!item) return;
    dialog.current?.close();
    router.push(item.href);
  };

  return (
    <>
      <button type="button" className="search-btn" onClick={open} aria-label="Search repositories, runs and steps" aria-keyshortcuts="Meta+K Control+K /">
        <Icon name="search" size={14} />
        <span>Search repositories, steps…</span>
        <span className="kbd" aria-hidden="true">
          {mac ? "⌘K" : "Ctrl K"}
        </span>
      </button>
      <dialog
        ref={dialog}
        className="cmdk"
        aria-label="Command menu"
        onClick={(e) => {
          if (e.target === dialog.current) dialog.current.close();
        }}
      >
        <div className="cmdk-input">
          <Icon name="search" size={16} />
          <input
            ref={input}
            value={q}
            onChange={(e) => {
              setQ(e.target.value);
              setActive(0);
            }}
            onKeyDown={(e) => {
              if (e.key === "ArrowDown") {
                e.preventDefault();
                setActive((a) => Math.min(items.length - 1, a + 1));
              } else if (e.key === "ArrowUp") {
                e.preventDefault();
                setActive((a) => Math.max(0, a - 1));
              } else if (e.key === "Enter") {
                e.preventDefault();
                go(items[active]);
              }
            }}
            placeholder="Jump to a repository, run or step…"
            role="combobox"
            aria-expanded="true"
            aria-controls={listId}
            aria-activedescendant={items.length ? `${listId}-${active}` : undefined}
            aria-autocomplete="list"
            autoComplete="off"
            spellCheck={false}
          />
          <span className="kbd">esc</span>
        </div>
        <div className="cmdk-list" id={listId} role="listbox" aria-label="Results">
          {items.length === 0 ? <div className="cmdk-empty">Nothing matches “{q}”.</div> : null}
          {items.map((item, i) => {
            const head = i === 0 || items[i - 1].group !== item.group ? item.group : null;
            return (
              <div key={item.id} role="presentation">
                {head ? (
                  <div className="menu-label" role="presentation">
                    {head}
                  </div>
                ) : null}
                <div
                  id={`${listId}-${i}`}
                  role="option"
                  aria-selected={i === active}
                  data-active={i === active}
                  className="menu-item"
                  onMouseMove={() => setActive(i)}
                  onClick={() => go(item)}
                >
                  {item.tone ? <Dot tone={item.tone === "gray" ? "white" : item.tone} size="sm" /> : null}
                  <span className="ellipsis" style={{ flex: 1, minWidth: 0, fontFamily: item.group === "Steps" ? "var(--mono)" : undefined, fontSize: item.group === "Steps" ? 13 : undefined }}>
                    {item.label}
                  </span>
                  {item.hint ? <span style={{ fontSize: 12, color: "var(--faint)", flex: "none" }}>{item.hint}</span> : null}
                </div>
              </div>
            );
          })}
        </div>
        <div className="cmdk-foot" aria-hidden="true">
          <span>↑↓ to move</span>
          <span>↵ to open</span>
          <span>esc to close</span>
        </div>
      </dialog>
    </>
  );
}
