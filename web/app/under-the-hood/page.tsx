import type { Metadata } from "next";
import { Code } from "@/components/code";
import { JoinForm } from "@/components/join-form";
import { ManifestDemo } from "@/components/manifest-demo";
import { RunSequence } from "@/components/run-sequence";
import { SiteFooter, SiteNav } from "@/components/site-nav";

export const metadata: Metadata = {
  title: "Under the hood: Quod",
  description:
    "How Quod decides a CI step can be skipped: a FUSE observer, BLAKE3 manifests and a trust layer, written in Rust.",
};

const CRATES: [string, string][] = [
  ["sp-cli", "The quod command: run, inspect, status, bench."],
  ["sp-fs", "The FUSE observer, over /dev/fuse or io_uring."],
  ["sp-sandbox", "bubblewrap sandbox, overlay and pinned root filesystem."],
  ["sp-cas", "BLAKE3 content-addressed store and the tree model."],
  ["sp-key", "Step keys and the Go, Node and Python profiles."],
  ["sp-manifest", "Manifests, results and matches()."],
  ["sp-trust", "Proof streaks, audits and quarantine."],
  ["sp-ledger", "SQLite ledger with versioned migrations."],
  ["sp-replay", "Restores outputs, stdout, stderr and exit code."],
  ["sp-proto", "Host-to-guest messages for the microVM backend (in progress)."],
  ["sp-guest-agent", "Runs a step inside a microVM guest (in progress)."],
  ["sp-uring-probe", "Prototype that measured FUSE over io_uring first."],
];

const LOOKUP = `pub(crate) fn lookup(
    &self, req: &Req, parent: u64, name: &[u8],
) -> Result<(u64, FileAttr), i32> {
    let p = self.node(parent).ok_or(libc::ENOENT)?;
    let found = /* binary search p.children for name */;
    match found {
        Some(ino) => {
            let n = &self.nodes[ino as usize - 1];
            self.record(req, ino, n.entry.k);
            Ok((ino, self.attr(ino, n)))
        }
        None => {
            // The step asked for a path that doesn't exist.
            // That's an input too: if it appears later,
            // the step may behave differently.
            let name = String::from_utf8_lossy(name);
            self.record_absent(req, child_path(&p.entry.p, &name));
            Err(libc::ENOENT)
        }
    }
}`;

const READDIR = `pub(crate) fn readdir(
    &self, req: &Req, ino: u64, offset: u64,
) -> Result<Vec<DirEnt<'_>>, i32> {
    let n = self.node(ino).ok_or(libc::ENOENT)?;
    // Listing a directory records the listing itself, hashed.
    // A new file in a globbed directory changes the result
    // even if nothing opens it.
    self.record(req, ino, Kind::Dir);
    // ...
}`;

const MTIME = `/// The modification time served for a regular file: a function of its
/// content alone, so it changes exactly when the content does.
pub fn content_mtime(content: &sp_cas::Hash) -> std::time::SystemTime {
    let b = content.as_bytes();
    let secs = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) & ((1 << 28) - 1);
    let nanos = u32::from_le_bytes([b[4], b[5], b[6], b[7]]) % 1_000_000_000;
    std::time::UNIX_EPOCH + std::time::Duration::new(946_684_800 + u64::from(secs), nanos)
}`;

const NEEDED = `/// Identical executions a key's manifests need: the policy's count plus one
/// for every divergence among the key's newest audits, at most MAX_EXTRA_PROOFS more.
pub fn needed(policy: &Policy, recent_divergences: u64) -> u32 {
    let extra = u32::try_from(recent_divergences)
        .unwrap_or(u32::MAX)
        .min(MAX_EXTRA_PROOFS);
    policy.proofs.saturating_add(extra)
}`;

const ENTRY_AGREES = `fn entry_agrees(want: &Entry, have: Option<&Entry>) -> bool {
    match (want.k, have) {
        (Kind::Absent, None) => true,
        (Kind::Absent, Some(_)) | (_, None) => false,
        (Kind::File, Some(h)) => h.k == Kind::File && h.h == want.h && h.m == want.m,
        (Kind::Dir, Some(h)) => h.k == Kind::Dir && h.h == want.h,
        (Kind::Symlink, Some(h)) => h.k == Kind::Symlink && h.t == want.t,
        (Kind::Deleted, Some(_)) => false,
    }
}`;

export default function UnderTheHood() {
  return (
    <>
      <SiteNav />
      <main>
        <section className="hero hood-hero">
          <div className="hero-txt">
            <h1>Under the hood.</h1>
            <p className="lede">
              Quod is about 19,000 lines of Rust in twelve crates. This page follows one CI step through it, then digs
              into the parts that were hard to get right.
            </p>
          </div>
        </section>

        <section className="sec" id="sequence">
          <div className="sec-head">
            <h2>One run, start to finish.</h2>
            <p className="lede">
              Every <code className="ic">quod run</code> goes through these seven stages. Click one to stop and read.
            </p>
          </div>
          <RunSequence />
        </section>

        <section className="sec split" id="manifest">
          <div className="sec-head">
            <h2>The manifest is the whole idea.</h2>
            <p className="lede">
              Nobody writes down a step&rsquo;s inputs. Quod records them while the step runs. Three kinds of entry
              cover everything a program can learn from a filesystem:
            </p>
            <ul className="points">
              <li>
                <b>f</b>: a file it read, by content hash and mode.
              </li>
              <li>
                <b>d</b>: a directory it listed, by a hash of the listing.
              </li>
              <li>
                <b>a</b>: a path it looked for and didn&rsquo;t find.
              </li>
            </ul>
          </div>
          <ManifestDemo />
        </section>

        <section className="sec" id="hard">
          <div className="sec-head">
            <h2>The hard parts.</h2>
            <p className="lede">
              A tool like this has one unforgivable bug: skipping a step whose result would have been different. Each of
              these is a way that almost happened, and what we do about it.
            </p>
          </div>

          <article className="hard">
            <div className="hard-txt">
              <h3>Recording what isn&rsquo;t there</h3>
              <p>
                Go checks for <code className="ic">go.work</code>. Node walks up looking for{" "}
                <code className="ic">node_modules</code>. Python searches a dozen paths for each import. If a step
                looked for a file and didn&rsquo;t find it, creating that file later can change its result. So a failed
                lookup is recorded as an input, just like a read.
              </p>
            </div>
            <Code src={LOOKUP} file="crates/sp-fs/src/serve.rs" hl={[12, 13, 14, 15, 16, 17]} />
          </article>

          <article className="hard">
            <div className="hard-txt">
              <h3>A directory listing is an input</h3>
              <p>
                <code className="ic">go test ./...</code> lists directories to find packages. Adding a new{" "}
                <code className="ic">_test.go</code> file changes what runs, even though nothing opened it yet. The
                observer records each listing it serves, and the manifest stores a hash of the names in it.
              </p>
            </div>
            <Code src={READDIR} file="crates/sp-fs/src/serve.rs" hl={[8]} />
          </article>

          <article className="hard">
            <div className="hard-txt">
              <h3>Timestamps would lie</h3>
              <p>
                Tools like <code className="ic">eslint --cache</code> and Python&rsquo;s bytecode cache trust a
                file&rsquo;s modification time. A fresh checkout has new timestamps every time, and a fixed fake one
                would let a cache serve stale results. So Quod serves an mtime computed from the file&rsquo;s content: it
                changes exactly when the bytes do.
              </p>
            </div>
            <Code src={MTIME} file="crates/sp-fs/src/lib.rs" hl={[5, 6, 7]} />
          </article>

          <article className="hard">
            <div className="hard-txt">
              <h3>Matching is strict, on purpose</h3>
              <p>
                There&rsquo;s no &ldquo;close enough&rdquo;. A file must have the same hash and mode, a listing the same
                names, a missing path must still be missing. One disagreement anywhere and the step runs, and Quod tells
                you which path moved.
              </p>
            </div>
            <Code src={ENTRY_AGREES} file="crates/sp-manifest/src/lib.rs" />
          </article>

          <article className="hard">
            <div className="hard-txt">
              <h3>The kernel is the bottleneck</h3>
              <p>
                Every file a step touches is a round trip from the kernel to Quod&rsquo;s process. A Go build on cli/cli
                makes tens of thousands of them. Classic FUSE answers each through a <code className="ic">read</code> and a{" "}
                <code className="ic">write</code> on <code className="ic">/dev/fuse</code>. Quod also speaks FUSE over
                io_uring (Linux 6.14+): one thread per CPU, pinned, answering from shared rings, with file contents passed
                straight through to the kernel.
              </p>
              <p className="faint small">
                It falls back to classic FUSE if the ring can&rsquo;t come up, and both transports must produce
                byte-identical access logs before anything merges.
              </p>
            </div>
            <div className="bench">
              <p className="bench-cap">Recorded run vs plain run on cli/cli, median of 6</p>
              <div className="bench-row">
                <span>Classic FUSE</span>
                <span className="bench-bar">
                  <i style={{ width: "100%" }} />
                </span>
                <b>+59%</b>
              </div>
              <div className="bench-row">
                <span>io_uring</span>
                <span className="bench-bar">
                  <i style={{ width: "77%" }} />
                </span>
                <b>+45%</b>
              </div>
              <div className="bench-row">
                <span>io_uring, pinned threads</span>
                <span className="bench-bar">
                  <i style={{ width: "37%" }} />
                </span>
                <b>+22%</b>
              </div>
              <p className="bench-note">
                That&rsquo;s the cost of the first, recorded run. Our target is under 10% and we&rsquo;re not there yet.
                A replay skips all of it.
              </p>
            </div>
          </article>

          <article className="hard">
            <div className="hard-txt">
              <h3>When in doubt, run it</h3>
              <p>
                Under-recording is the only failure that could cause a wrong skip, so Quod records too much rather than
                too little. Any I/O error inside the observer aborts the run with exit code 2 instead of saving a
                partial manifest. Steps that use the network are never replayed. Failures and timed-out runs never count
                as proof. And a result has to come out identical at least twice before it can be skipped, with every
                divergence raising that bar for the whole key.
              </p>
            </div>
            <Code src={NEEDED} file="crates/sp-trust/src/lib.rs" hl={[4, 5, 6, 7]} />
          </article>
        </section>

        <section className="sec" id="tested">
          <div className="proof">
            <div>
              <span className="big">0</span>
              <p>wrong skips on cli/cli after changing each of 23,939 files, 5,320 directories and 621 missing paths, one at a time</p>
            </div>
            <div>
              <span className="big">2</span>
              <p>FUSE transports, required to log every access identically</p>
            </div>
            <div>
              <span className="big">13</span>
              <p>crash, fault-injection and transport tests that must pass before anything merges</p>
            </div>
          </div>
        </section>

        <section className="sec" id="crates">
          <div className="sec-head">
            <h2>The crates.</h2>
            <p className="lede">Each one does one job and knows as little as possible about the others.</p>
          </div>
          <ul className="crates">
            {CRATES.map(([name, what]) => (
              <li key={name}>
                <code>{name}</code>
                <span>{what}</span>
              </li>
            ))}
          </ul>
        </section>

        <section className="cta" id="join">
          <h2>Want it on your CI?</h2>
          <p className="lede">We&rsquo;re letting teams in a few at a time. Join the waitlist.</p>
          <JoinForm source="under-the-hood" />
        </section>
      </main>
      <SiteFooter />
    </>
  );
}
