"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { DIM, HEAD, MiniShell, QUOD_HELP, REPO, createQuod } from "@/lib/quod-demo";
import { sleep, useInView, useReducedMotion } from "@/lib/use-in-view";

type Line = { kind: "cmd" | "out"; text: string };
type Exec = (cmd: string) => Promise<string>;

// What the terminal does on its own: test, replay, prove an unrelated edit is
// harmless, then break the code and watch Quod name the file.
const SCRIPT = [
  "quod run go test ./...",
  "quod run go test ./...",
  'echo "Now with docs." >> README.md',
  "quod run go test ./...",
  "sed -i 's/a + b/a - b/' calc/calc.go",
  "quod run go test ./...",
  "sed -i 's/a - b/a + b/' calc/calc.go",
  "quod run go test ./...",
];

const CHIPS: [string, string][] = [
  ["Run the tests", "quod run go test ./..."],
  ["Edit the README", 'echo "Now with docs." >> README.md'],
  ["Change the code", "sed -i 's/a + b/a - b/' calc/calc.go"],
  ["Fix it", "sed -i 's/a - b/a + b/' calc/calc.go"],
  ["Add a test case", "echo c > calc/testdata/cases/c.txt"],
  ["Create a probed file", 'echo "{}" > calc/testdata/override.json'],
  ["What did it read?", "quod inspect"],
];

const HELP = `Try these, or any shell command:
  quod run go test ./...
  echo "more" >> README.md
  sed -i 's/a + b/a - b/' calc/calc.go
  echo "{}" > calc/testdata/override.json
  quod inspect
`;

async function makeShell(): Promise<{ exec: Exec; engine: string }> {
  try {
    const mod = await import("just-bash/browser");
    const files = Object.fromEntries(Object.entries(REPO).map(([k, v]) => [`/repo/${k}`, v]));
    // eslint-disable-next-line prefer-const -- assigned after the command that closes over it
    let bash: InstanceType<typeof mod.Bash>;
    const quod = createQuod({
      readFile: (p) => bash.fs.readFile(p),
      exists: (p) => bash.fs.exists(p),
      readdir: async (p) => [...(await bash.fs.readdir(p))].sort(),
      isDir: async (p) => {
        try {
          await bash.fs.readdir(p);
          return true;
        } catch {
          return false;
        }
      },
    });
    const cmd = mod.defineCommand("quod", async (args) => {
      if (args[0] === "run") return quod.run(args.slice(1));
      if (args[0] === "inspect") return { stdout: await quod.inspect(), stderr: "", exitCode: 0 };
      return { stdout: QUOD_HELP, stderr: "", exitCode: 0 };
    });
    bash = new mod.Bash({ files, cwd: "/repo", customCommands: [cmd] });
    return {
      exec: async (c) => {
        const r = await bash.exec(c);
        return (r.stdout ?? "") + (r.stderr ?? "");
      },
      engine: "just-bash",
    };
  } catch {
    const sh = new MiniShell();
    return { exec: (c) => sh.exec(c), engine: "demo shell" };
  }
}

function Output({ text }: { text: string }) {
  return (
    <>
      {text.split("\n").map((l, i, all) => {
        if (i === all.length - 1 && l === "") return null;
        if (l.startsWith(HEAD))
          return (
            <span key={i} className="hl">
              {l.slice(1)}
              {"\n"}
            </span>
          );
        if (l.startsWith(DIM))
          return (
            <span key={i} className="dim">
              {l.slice(1)}
              {"\n"}
            </span>
          );
        return <span key={i}>{l + "\n"}</span>;
      })}
    </>
  );
}

export function Terminal({ id }: { id: string }) {
  const [ref, inView] = useInView<HTMLDivElement>(0.45);
  const reduce = useReducedMotion();
  const [lines, setLines] = useState<Line[]>([]);
  const [draft, setDraft] = useState("");
  const [engine, setEngine] = useState("loading");
  const [driving, setDriving] = useState(false);
  const shell = useRef<Exec | null>(null);
  const outRef = useRef<HTMLDivElement>(null);
  const history = useRef<string[]>([]);
  const hi = useRef(0);

  const reset = useCallback(async () => {
    const s = await makeShell();
    shell.current = s.exec;
    setEngine(s.engine);
    setLines([
      {
        kind: "out",
        text: `${DIM}A small Go project with the quod CLI, running in your browser.\n${DIM}It runs by itself. Click anywhere to try it.\n\n`,
      },
    ]);
  }, []);

  useEffect(() => {
    reset();
  }, [reset]);

  useEffect(() => {
    const el = outRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [lines]);

  const run = useCallback(async (c: string) => {
    if (!shell.current) return;
    setLines((l) => [...l, { kind: "cmd", text: c }]);
    const t = c.trim();
    if (t === "clear") return setLines([]);
    const text = t === "help" ? HELP : await shell.current(c).catch((e) => `${String(e?.message ?? e)}\n`);
    setLines((l) => [...l, { kind: "out", text }]);
  }, []);

  // Self-running script while on screen, until the visitor takes over.
  useEffect(() => {
    if (!inView || driving || reduce || engine === "loading") return;
    const ctl = new AbortController();
    (async () => {
      for (;;) {
        for (const c of SCRIPT) {
          for (let i = 1; i <= c.length; i++) {
            setDraft(c.slice(0, i));
            await sleep(26, ctl.signal);
          }
          await sleep(260, ctl.signal);
          setDraft("");
          await run(c);
          await sleep(c.startsWith("quod") ? 1700 : 700, ctl.signal);
        }
        await sleep(3500, ctl.signal);
        await reset();
      }
    })().catch(() => {});
    return () => {
      ctl.abort();
      setDraft("");
    };
  }, [inView, driving, reduce, engine, run, reset]);

  const takeOver = () => {
    if (!driving) {
      setDriving(true);
      setDraft("");
    }
  };

  return (
    <div className="term" ref={ref} onPointerDown={takeOver}>
      <div className="term-head">
        <span>~/tiny</span>
        <span>{driving ? "you're driving. Type help." : engine === "loading" ? "loading shell" : `playing on its own, ${engine}`}</span>
      </div>
      <div className="term-out" ref={outRef} tabIndex={0} aria-live="polite" aria-label="Terminal output">
        {lines.map((l, i) =>
          l.kind === "cmd" ? (
            <span key={i} className="cmd">
              $ {l.text}
              {"\n"}
            </span>
          ) : (
            <Output key={i} text={l.text} />
          ),
        )}
      </div>
      <label className="term-in" htmlFor={id}>
        <span aria-hidden>$</span>
        <input
          id={id}
          value={draft}
          autoComplete="off"
          spellCheck={false}
          aria-label="Type a command"
          placeholder={driving ? "" : "click to take over"}
          onFocus={takeOver}
          onChange={(e) => {
            takeOver();
            setDraft(e.target.value);
          }}
          onKeyDown={async (e) => {
            if (e.key === "Enter" && draft.trim()) {
              const c = draft;
              setDraft("");
              history.current.push(c);
              hi.current = history.current.length;
              await run(c);
            } else if (e.key === "ArrowUp" && hi.current > 0) {
              e.preventDefault();
              setDraft(history.current[--hi.current]);
            } else if (e.key === "ArrowDown" && hi.current < history.current.length) {
              e.preventDefault();
              setDraft(history.current[++hi.current] ?? "");
            }
          }}
        />
      </label>
      <div className="chips">
        {CHIPS.map(([label, c]) => (
          <button
            key={label}
            type="button"
            title={c}
            onClick={() => {
              takeOver();
              run(c);
            }}
          >
            {label}
          </button>
        ))}
      </div>
    </div>
  );
}
