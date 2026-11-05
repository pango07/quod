"use client";

import { useEffect, useRef, useState } from "react";
import { DIM, HEAD, MiniShell, QUOD_HELP, REPO, createQuod } from "@/lib/quod-demo";
import { sleep } from "@/lib/use-in-view";

type Line = { kind: "cmd" | "out"; text: string };
type Exec = (cmd: string) => Promise<string>;

// Optional guided playback: record, confirm, reuse, then edit and repair the example.
const SCRIPT = [
  "npm test",
  "npm test",
  "npm test",
  'echo "Now with docs." >> README.md',
  "npm test",
  "sed -i 's/a + b/a - b/' calc/calc.ts",
  "npm test",
  "sed -i 's/a - b/a + b/' calc/calc.ts",
  "npm test",
];

const CHIPS: [string, string][] = [
  ["Run the tests", "npm test"],
  ["Edit the README", 'echo "Now with docs." >> README.md'],
  ["Change the code", "sed -i 's/a + b/a - b/' calc/calc.ts"],
  ["Fix it", "sed -i 's/a - b/a + b/' calc/calc.ts"],
  ["Add a fixture", "echo c > calc/testdata/cases/c.txt"],
  ["Create a probed file", 'echo "{}" > calc/testdata/override.json'],
  ["What did it read?", "quod inspect"],
];

const HELP = `Try these commands in the in-memory example:
  npm test
  echo "more" >> README.md
  sed -i 's/a + b/a - b/' calc/calc.ts
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
    bash = new mod.Bash({ files, cwd: "/repo", customCommands: [cmd, mod.defineCommand("npm", args => quod.run(["npm", ...args]))] });
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

const GREETING: Line[] = [{ kind: "out", text: `${DIM}Interactive Vitest example · illustrative model\n${DIM}Commands edit an in-memory project. Quod behavior and test outcomes are simulated.\n${DIM}Run twice to record and confirm, then run again to reuse.\n\n` }];

export function Terminal({ id }: { id: string }) {
  const [lines, setLines] = useState<Line[]>(GREETING);
  const [draft, setDraft] = useState("");
  const [ready, setReady] = useState(false);
  const [busy, setBusy] = useState(false);
  const [playing, setPlaying] = useState(false);
  const shell = useRef<Exec | null>(null);
  const locked = useRef(false);
  const outRef = useRef<HTMLDivElement>(null);
  const history = useRef<string[]>([]);
  const hi = useRef(0);
  const playback = useRef<AbortController | null>(null);

  useEffect(() => {
    let active = true;
    makeShell().then(s => {
      if (!active) return;
      shell.current = s.exec;
      setReady(true);
    });
    return () => { active = false; playback.current?.abort(); };
  }, []);

  useEffect(() => {
    const el = outRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [lines]);

  async function run(command: string) {
    if (!shell.current || locked.current) return;
    locked.current = true;
    setBusy(true);
    try {
      setLines(value => [...value, { kind: "cmd", text: command }]);
      const text = command.trim() === "help" ? HELP : await shell.current(command);
      if (command.trim() === "clear") setLines([]);
      else setLines(value => [...value, { kind: "out", text }]);
    } catch (error) {
      setLines(value => [...value, { kind: "out", text: `${String(error)}\n` }]);
    } finally {
      locked.current = false;
      setBusy(false);
    }
  }

  async function reset() {
    setReady(false);
    const s = await makeShell();
    shell.current = s.exec;
    history.current = [];
    hi.current = 0;
    setDraft("");
    setLines(GREETING);
    setReady(true);
  }

  async function play() {
    const ctl = new AbortController();
    playback.current = ctl;
    setPlaying(true);
    try {
      await reset();
      for (const command of SCRIPT) {
        if (ctl.signal.aborted) break;
        await run(command);
        await sleep(1100, ctl.signal);
      }
    } catch (error) {
      if (!ctl.signal.aborted) setLines(value => [...value, { kind: "out", text: `${String(error)}\n` }]);
    } finally {
      setPlaying(false);
      playback.current = null;
    }
  }

  return (
    <div className="term">
      <div className="term-head">
        <span>~/tiny · browser playground</span>
        <div className="demo-toolbar">
          <button className="demo-control" type="button" disabled={!ready || (!playing && busy)} onClick={() => playing ? playback.current?.abort() : void play()}>{playing ? "Stop demo" : "Play example"}</button>
          <button className="demo-control" type="button" disabled={!ready || busy || playing} onClick={reset}>Reset terminal</button>
        </div>
      </div>
      <div className="term-out" ref={outRef} tabIndex={0} aria-live={playing ? "off" : "polite"} aria-label="Terminal output">
        {lines.map((line, i) => line.kind === "cmd" ? <span key={i} className="cmd">$ {line.text}{"\n"}</span> : <Output key={i} text={line.text} />)}
      </div>
      <form onSubmit={async event => {
        event.preventDefault();
        if (!draft.trim() || busy || playing || !ready) return;
        const command = draft;
        setDraft("");
        history.current.push(command);
        hi.current = history.current.length;
        await run(command);
      }}>
        <label className="term-in" htmlFor={id}>
          <span aria-hidden>$</span>
          <input id={id} value={draft} disabled={!ready || busy || playing} autoComplete="off" spellCheck={false} aria-label="Type a command" placeholder={ready ? "npm test, or type help" : "loading shell…"} onChange={event => setDraft(event.target.value)} onKeyDown={event => {
            if (event.key === "ArrowUp" && hi.current > 0) { event.preventDefault(); setDraft(history.current[--hi.current]); }
            else if (event.key === "ArrowDown" && hi.current < history.current.length) { event.preventDefault(); setDraft(history.current[++hi.current] ?? ""); }
          }} />
          <button className="demo-control" type="submit" disabled={!ready || busy || playing || !draft.trim()}>Run</button>
        </label>
      </form>
      <div className="chips">
        {CHIPS.map(([label, command]) => <button key={label} type="button" title={command} disabled={!ready || busy || playing} onClick={() => run(command)}>{label}</button>)}
      </div>
    </div>
  );
}
