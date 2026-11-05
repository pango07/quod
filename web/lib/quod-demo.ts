// Illustrative browser model, not the Quod recorder or a Vitest runtime.
// Shell edits are real in-memory edits; test outcomes and input reads are modeled.

export const REPO: Record<string, string> = {
  "package.json": '{"name":"tiny","scripts":{"test":"vitest run"}}\n',
  "README.md": "# tiny\n\nA small calculator.\n",
  "calc/calc.ts": "export function add(a: number, b: number) { return a + b; }\n",
  "calc/calc.test.ts": 'import { expect, test } from "vitest";\nimport { add } from "./calc";\ntest("adds", () => expect(add(2, 3)).toBe(5));\n',
  "calc/testdata/fixture.txt": "hello\n",
  "calc/testdata/cases/a.txt": "a\n",
  "calc/testdata/cases/b.txt": "b\n",
  "docs/guide.md": "# Guide\n\nUsage notes.\n",
};

type Kind = "f" | "d" | "a";
const READS: [Kind, string][] = [
  ["d", "."],
  ["f", "package.json"],
  ["d", "calc"],
  ["f", "calc/calc.ts"],
  ["f", "calc/calc.test.ts"],
  ["d", "calc/testdata"],
  ["f", "calc/testdata/fixture.txt"],
  ["d", "calc/testdata/cases"],
  ["a", "calc/testdata/override.json"],
];

/** The filesystem the demo needs, whichever shell provides it. */
export interface DemoFs {
  readFile(path: string): Promise<string>;
  exists(path: string): Promise<boolean>;
  isDir(path: string): Promise<boolean>;
  readdir(path: string): Promise<string[]>;
}

export interface ExecResult {
  stdout: string;
  stderr: string;
  exitCode: number;
}

function hash(s: string): string {
  let a = 0x811c9dc5;
  let b = 0x01000193;
  for (let i = 0; i < s.length; i++) {
    const c = s.charCodeAt(i);
    a = Math.imul(a ^ c, 16777619) >>> 0;
    b = Math.imul(b ^ c, 2246822519) >>> 0;
  }
  return a.toString(16).padStart(8, "0") + b.toString(16).padStart(8, "0");
}

const runId = () =>
  ("01" + Date.now().toString(32).toUpperCase() + Math.random().toString(36).slice(2, 10).toUpperCase()).slice(0, 26);

/** Marks a line as the banner head or as dim text for the terminal renderer. */
export const HEAD = "\u0001";
export const DIM = "\u0002";

interface Manifest {
  entries: [Kind, string, string][];
  id: string;
  run: string;
  out: string;
  exit: number;
  confirmations: number;
}

export const QUOD_HELP = `npm test                 simulate recording, confirmation or reuse
quod inspect             show what the last run read
`;

export function createQuod(fs: DemoFs) {
  const ledger: Manifest[] = [];

  async function entry(kind: Kind, rel: string): Promise<string> {
    const abs = "/repo" + (rel === "." ? "" : "/" + rel);
    if (kind === "a") return (await fs.exists(abs)) ? "present" : "absent";
    if (kind === "d") {
      if (!(await fs.isDir(abs))) return "missing";
      const lines: string[] = [];
      for (const n of await fs.readdir(abs)) lines.push(`${n}\t${(await fs.isDir(`${abs}/${n}`)) ? "d" : "f"}`);
      return hash(lines.join("\n"));
    }
    try {
      return hash(await fs.readFile(abs));
    } catch {
      return "missing";
    }
  }

  async function snapshot(): Promise<[Kind, string, string][]> {
    const out: [Kind, string, string][] = [];
    for (const [k, p] of READS) out.push([k, p, await entry(k, p)]);
    return out;
  }

  async function test(): Promise<{ out: string; exit: number }> {
    const code = await fs.readFile("/repo/calc/calc.ts").catch(() => "");
    const fixture = await fs.readFile("/repo/calc/testdata/fixture.txt").catch(() => "");
    const override = await fs.exists("/repo/calc/testdata/override.json");
    if (!fixture.includes("hello"))
      return { out: "FAIL calc/calc.test.ts — fixture must contain hello\n", exit: 1 };
    if (!/return\s+a\s*\+\s*b/.test(code))
      return {
        out: "FAIL calc/calc.test.ts — expected add(2, 3) to be 5\n",
        exit: 1,
      };
    return {
      out: (override ? "    using testdata/override.json\n" : "") + "PASS calc/calc.test.ts — modeled test outcomes agree\n",
      exit: 0,
    };
  }

  const why = (k: Kind, p: string) =>
    k === "d" ? `${p}: directory listing changed` : k === "a" ? `${p}: was absent, now present` : `${p}: file content changed`;

  return {
    async run(args: string[]): Promise<ExecResult> {
      const cmd = args.join(" ").replace(/^--\s*/, "");
      if (cmd !== "npm test")
        return { stdout: "", stderr: "This demo models npm test; it does not execute Vitest.\n", exitCode: 2 };
      const now = await snapshot();
      const key = hash("step:" + cmd).slice(0, 8);
      let closest: [Kind, string, string] | null = null;
      for (const m of [...ledger].reverse()) {
        const diff = m.entries.find((e, i) => e[2] !== now[i][2]);
        if (!diff && m.exit === 0) {
          if (m.confirmations < 2) {
            m.confirmations += 1;
            return { stdout: `${HEAD}QUOD  CONFIRMED\n${DIM}  Another modeled execution agrees. This record is eligible for reuse.\n${m.out}`, stderr: "", exitCode: 0 };
          }
          return {
            stdout:
              `${HEAD}QUOD  REPLAYED\n${DIM}  modeled record ${m.run.slice(0, 8)}\n` +
              `${DIM}  step ${key}  manifest ${m.id.slice(0, 8)}  ${now.length} inputs matched\n` +
              m.out,
            stderr: "",
            exitCode: m.exit,
          };
        }
        closest ??= diff ?? null;
      }
      const res = await test();
      const id = hash(JSON.stringify(now));
      ledger.push({ entries: now, id, run: runId(), out: res.out, exit: res.exit, confirmations: 1 });
      const reason = closest
        ? `${DIM}  closest manifest differed at ${closest[1]}\n${DIM}    ${why(closest[0], closest[1])}\n`
        : `${DIM}  no manifests for this step key\n`;
      return {
        stdout:
          `${HEAD}QUOD  EXECUTED\n${DIM}  step ${key}  manifest ${id.slice(0, 8)}  ${now.length} inputs recorded\n` +
          reason +
          `${DIM}  Illustrative execution; no timing measurement.\n` +
          res.out,
        stderr: "",
        exitCode: res.exit,
      };
    },
    async inspect(): Promise<string> {
      const m = ledger.at(-1);
      if (!m) return "no runs yet. Try: npm test\n";
      return (
        `manifest ${m.id.slice(0, 8)}, newest of ${ledger.length}:\n` +
        m.entries.map(([k, p, h]) => `  ${k}  ${p.padEnd(30)} ${h.length > 12 ? h.slice(0, 8) : h}`).join("\n") +
        "\n"
      );
    },
  };
}

/** An in-memory filesystem and a few commands, for when just-bash can't load. */
export class MiniShell implements DemoFs {
  files = new Map(Object.entries(REPO).map(([k, v]) => [`/repo/${k}`, v]));
  quod = createQuod(this);

  private norm(p: string): string {
    const parts: string[] = [];
    for (const s of (p.startsWith("/") ? p : `/repo/${p}`).split("/")) {
      if (!s || s === ".") continue;
      if (s === "..") parts.pop();
      else parts.push(s);
    }
    return "/" + parts.join("/");
  }
  async readFile(p: string) {
    const v = this.files.get(p);
    if (v === undefined) throw new Error("ENOENT");
    return v;
  }
  async exists(p: string) {
    return this.files.has(p) || (await this.isDir(p));
  }
  async isDir(p: string) {
    for (const k of this.files.keys()) if (k.startsWith(p + "/")) return true;
    return false;
  }
  async readdir(p: string) {
    const set = new Set<string>();
    for (const k of this.files.keys()) if (k.startsWith(p + "/")) set.add(k.slice(p.length + 1).split("/")[0]);
    return [...set].sort();
  }

  async exec(line: string): Promise<string> {
    const toks = (line.match(/"[^"]*"|'[^']*'|>>|>|\S+/g) ?? []).map((t) =>
      /^["'].*["']$/.test(t) ? t.slice(1, -1) : t,
    );
    let redirect: string | null = null;
    let append = false;
    for (const op of [">>", ">"]) {
      const i = toks.indexOf(op);
      if (i >= 0) {
        redirect = toks[i + 1] ?? null;
        append = op === ">>";
        toks.splice(i, 2);
        break;
      }
    }
    const [cmd, ...args] = toks;
    let out = "";
    switch (cmd) {
      case "ls":
        out = (await this.readdir(this.norm(args[0] ?? "."))).join("  ") + "\n";
        break;
      case "cat":
        for (const a of args) out += await this.readFile(this.norm(a)).catch(() => `cat: ${a}: No such file or directory\n`);
        break;
      case "echo":
        out = args.join(" ") + "\n";
        break;
      case "touch":
        for (const a of args) if (!(await this.exists(this.norm(a)))) this.files.set(this.norm(a), "");
        break;
      case "rm":
        for (const a of args) this.files.delete(this.norm(a));
        break;
      case "sed": {
        const expr = args[args.indexOf("-i") + 1] ?? "";
        const file = this.norm(args.at(-1) ?? "");
        const m = /^s\/(.*?)\/(.*?)\/(g?)$/.exec(expr);
        if (!m) {
          out = "sed: only s/old/new/ is supported here\n";
          break;
        }
        const re = new RegExp(m[1].replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), m[3] ? "g" : "");
        this.files.set(file, (await this.readFile(file).catch(() => "")).replace(re, m[2]));
        break;
      }
      case "npm": {
        const r = await this.quod.run(["npm", ...args]);
        out = r.stdout + r.stderr;
        break;
      }
      case "quod": {
        if (args[0] === "run") {
          const r = await this.quod.run(args.slice(1));
          out = r.stdout + r.stderr;
        } else if (args[0] === "inspect") out = await this.quod.inspect();
        else out = QUOD_HELP;
        break;
      }
      case "pwd":
        out = "/repo\n";
        break;
      case undefined:
        break;
      default:
        out = `${cmd}: not available in this demo shell\n`;
    }
    if (redirect) {
      const p = this.norm(redirect);
      this.files.set(p, (append ? await this.readFile(p).catch(() => "") : "") + out);
      return "";
    }
    return out;
  }
}
