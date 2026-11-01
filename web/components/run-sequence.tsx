"use client";

import { useEffect, useState } from "react";
import { Code } from "@/components/code";
import { sleep, useInView, useReducedMotion } from "@/lib/use-in-view";

// One `quod run`, in order. Excerpts are trimmed from the real source; `// ...` marks a cut.
const STAGES: { title: string; body: string; file: string; src: string; hl?: number[] }[] = [
  {
    title: "Name the step",
    body: "The command, its environment and the pinned root filesystem are hashed into a step key. Volatile variables like a timestamp or CI run number are refused, so the same step always gets the same key.",
    file: "crates/quod-key/src/lib.rs",
    src: `pub struct StepSpec {
    pub image: Image,          // rootfs digest + platform
    pub toolchain: Toolchain,  // Go, Node or Python profile
    pub cmd: Vec<String>,
    pub cwd: String,
    pub env: Vec<(String, String)>, // sorted by name in the key
    pub roots: Vec<String>,
    pub warm: bool,
}

pub fn step_key(spec: &StepSpec) -> Result<Hash, KeyError> {
    Ok(Hash::of(canonical_key_json(spec)?.as_bytes()))
}`,
    hl: [10, 11],
  },
  {
    title: "Snapshot the repo",
    body: "Every file in the checkout is hashed with BLAKE3 into a content-addressed store. The result is a tree: path to hash, mode and kind. Unchanged files are never hashed twice.",
    file: "crates/quod-cli/src/run.rs",
    src: `let key = step_key(&spec)?;
// ...
let tree = deps::import_workspace(&cas, &repo, &deps, spec.profile())?;`,
    hl: [3],
  },
  {
    title: "Look for a match",
    body: "Quod loads the manifests recorded for this key and checks each one against the new tree. A manifest only lists what the step read last time, so edits anywhere else don't matter.",
    file: "crates/quod-manifest/src/lib.rs",
    src: `pub fn matches(manifest: &Manifest, tree: &Tree) -> Result<(), Box<Mismatch>> {
    let mut first: Option<(String, Entry, Option<Entry>)> = None;
    let mut total = 0;
    for want in &manifest.entries {
        let have = tree.get(&want.p);
        if !entry_agrees(want, have) {
            total += 1;
            if first.is_none() {
                first = Some((want.p.clone(), want.clone(), have.cloned()));
            }
        }
    }
    // ...
}`,
    hl: [4, 5, 6],
  },
  {
    title: "Decide",
    body: "A match isn't enough. The result must have been produced identically at least twice, the key can't be quarantined, and a share of would-be skips still run as audits.",
    file: "crates/quod-trust/src/lib.rs",
    src: `pub fn decide(policy: &Policy, hit: Option<HitView>, trust: Option<&TrustRow>,
              proof: Option<&ProofRow>, recent_divergences: u64) -> Decision {
    let Some(hit) = hit else {
        return Decision::Execute(Reason::Miss);
    };
    // ... quarantined or unprovable keys execute ...
    if streak < needed {
        return Decision::Execute(Reason::Proving { streak, needed });
    }
    match policy.mode {
        Mode::Observe => Decision::Execute(Reason::WouldReplay),
        Mode::Replay => {
            let eligible = trust.map_or(0, |t| t.eligible);
            if audit_due(eligible, policy.audit) {
                Decision::Execute(Reason::Audit { nth: eligible + 1 })
            } else {
                Decision::Replay
            }
        }
    }
}`,
    hl: [7, 8, 9, 14, 15, 17],
  },
  {
    title: "Replay, or run under observation",
    body: "On a replay, Quod restores the recorded outputs, stdout, stderr and exit code. Otherwise the step runs in a sandbox whose /workspace is Quod's own FUSE filesystem, which logs every request the kernel sends it.",
    file: "crates/quod-fs/src/serve.rs",
    src: `pub(crate) fn open(&self, req: &Req, ino: u64, read_only: bool, /* ... */) -> Result<Opened, i32> {
    let n = self.node(ino).ok_or(libc::ENOENT)?;
    if n.entry.k != Kind::File {
        return Err(libc::EISDIR);
    }
    if !read_only {
        return Err(libc::EROFS); // writes go to an overlay above us
    }
    self.record(req, ino, Kind::File);
    // ...
}`,
    hl: [9],
  },
  {
    title: "Write the manifest",
    body: "At unmount, the access log becomes a manifest: every file read, every directory listed, every path looked up and not found. Writes are diffed out of the overlay as outputs.",
    file: "crates/quod-manifest/src/lib.rs",
    src: `/// What one execution of a step actually read. Entries are sorted
/// bytewise by path (then kind) and unique.
pub struct Manifest {
    pub v: u32,
    pub step_key: Hash,
    pub roots: Vec<String>,
    pub entries: Vec<Entry>, // f file, d dir listing, l symlink, a absent
}`,
    hl: [7],
  },
  {
    title: "Prove and record",
    body: "The new result is compared with the last one from the same manifest. Identical extends the proof streak; different quarantines the whole key. Verdict, run, manifest and result commit in one SQLite transaction.",
    file: "crates/quod-cli/src/run.rs",
    src: `// The trust update and the run row are one transaction.
ledger.atomically(|l| -> anyhow::Result<_> {
    Ok(match trust.record_replay(&tstep, hit)? {
        Replayed::Replayed(facts) => {
            l.record_run_facts(&info.facts("replayed", /* ... */))?;
            Ok((facts, replay_wall))
        }
        Replayed::Execute(reason) => Err(reason),
    })
})?`,
    hl: [1, 2],
  },
];

export function RunSequence() {
  const [ref, inView] = useInView<HTMLDivElement>(0.35);
  const reduce = useReducedMotion();
  const [on, setOn] = useState(0);
  const [held, setHeld] = useState(false);

  useEffect(() => {
    if (!inView || held || reduce) return;
    const ctl = new AbortController();
    (async () => {
      for (let i = on; ; i = (i + 1) % STAGES.length) {
        setOn(i);
        await sleep(5200, ctl.signal);
      }
    })().catch(() => {});
    return () => ctl.abort();
    // `on` is the starting point only; the loop owns it after that.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [inView, held, reduce]);

  const s = STAGES[on];
  return (
    <div className="seq" ref={ref} onMouseLeave={() => setHeld(false)}>
      <ol className="seq-list">
        {STAGES.map((st, i) => (
          <li key={st.title}>
            <button
              type="button"
              aria-current={i === on ? "step" : undefined}
              className={i === on ? "on" : i < on ? "past" : ""}
              onClick={() => {
                setHeld(true);
                setOn(i);
              }}
            >
              <span className="n">{i + 1}</span>
              {st.title}
              {i === on && !held && !reduce ? <i className="seq-bar" key={on} aria-hidden /> : null}
            </button>
          </li>
        ))}
      </ol>
      <div className="seq-detail" aria-live="polite">
        <p className="seq-body">{s.body}</p>
        <Code src={s.src} file={s.file} hl={s.hl} />
      </div>
    </div>
  );
}
