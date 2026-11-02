//! The trust layer: when a recorded result may be replayed, how a step key
//! earns that, and how it loses it.
//!
//! A matching manifest is necessary for a replay but not sufficient. The
//! result must also be *proven*: the same manifest must have produced the
//! same result `proofs` executions in a row (the proof streak, kept per
//! (key, manifest) in the ledger's `proofs` table). A result seen once is
//! never replayed. Nor, by default, is any failure (non-zero exit; a
//! failure can repeat identically, e.g. a test that hangs under load every
//! time the box is busy), nor any result whose output carries a kill or
//! timeout marker (`KILL_MARKERS`): such results are compared like any
//! other, but their streak never advances. `--replay-failures` lets
//! failures prove themselves like passes; kill markers never do.
//!
//! Every execution of a hit is compared with the result a replay would have
//! served (exit code, stdout and stderr up to `normalize`, outputs outside
//! the scratch directories the caller names: build cache, HOME) and must
//! have read an `equivalent` manifest. Any difference quarantines the whole
//! key: nothing of it replays until some manifest has been
//! reproduced `proofs + divergences` times in a row, and the divergence is
//! reported loudly. In observe mode every eligible hit is compared this way
//! (an audit for free, since the step executes anyway); in replay mode a
//! deterministic fraction of eligible hits is re-executed instead of
//! replayed. Audit never turns off unless the rate is set to 0 on purpose.
//!
//! The ledger stores the facts; this crate decides. Nothing here knows about
//! FUSE, sandboxes or the command line.

use std::collections::BTreeMap;
use std::fmt;
use std::io::Read;
use std::str::FromStr;
use std::time::Duration;

use quod_cas::{CanonError, Cas, CasError, Entry, Hash, Kind, parent_and_name};
use quod_ledger::{AuditRow, Ledger, LedgerError, ProofRow, TrustRow, TrustState, now_rfc3339};
use quod_manifest::{Manifest, ManifestError, StepResult, manifest_id};

/// Identical executions a manifest needs before its result may replay.
/// Two: a result seen once is never trusted.
pub const DEFAULT_PROOFS: u32 = 2;
/// The fraction of would-be hits replay mode re-executes and compares.
pub const DEFAULT_AUDIT: f64 = 0.05;
/// How many of the newest audits the trailing divergence rate covers, and
/// the window a key's divergences raise its bar within.
pub const TRAILING_AUDITS: usize = 100;
/// The most a key's recent divergences add to its bar: a chronically flaky
/// key needs at most `proofs + MAX_EXTRA_PROOFS` identical executions to
/// re-prove itself, and its divergences age out of the window.
pub const MAX_EXTRA_PROOFS: u32 = 5;
/// The largest `--proofs` accepted.
pub const MAX_PROOFS: u32 = 100;

/// Paths under HOME that tools fill with per-execution state and that are
/// therefore not compared (nor written out by `quod run --outputs`,
/// nor served by a replay of the runner hooks). Everything else a step
/// leaves in HOME is compared like any output: `go install` into a
/// HOME-relative GOPATH, a token, a generated config are results. Each entry
/// is here because cli/cli's tests write it differently on every run:
/// gh's HTTP cache (keys include random httptest ports), Go's telemetry
/// counters (date-stamped files), gh's state dir (a random device id; gh's
/// own directory only, not every tool's `.local/state`), and the SSH key
/// pair gh generates for codespaces. Each was seen differing between two
/// executions on identical inputs (exp/trust's P1 on cli/cli); nothing is
/// on the list without that.
pub const HOME_VOLATILE: &[&str] = &[
    ".cache/gh",
    ".config/go/telemetry",
    ".local/state/gh",
    ".ssh/codespaces.auto",
    ".ssh/codespaces.auto.pub",
];

/// `observe` executes everything and reports what would have replayed;
/// `replay` replays proven results.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Observe,
    Replay,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Observe => "observe",
            Mode::Replay => "replay",
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Mode {
    type Err = TrustError;
    fn from_str(s: &str) -> Result<Mode, TrustError> {
        match s {
            "observe" => Ok(Mode::Observe),
            "replay" => Ok(Mode::Replay),
            other => Err(TrustError::BadPolicy(format!(
                "mode {other:?} is neither observe nor replay"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Policy {
    pub mode: Mode,
    /// Identical executions a manifest needs (plus one per past divergence
    /// of its key) before it may replay. At least 1.
    pub proofs: u32,
    /// Fraction of would-be hits re-executed in replay mode, 0..=1.
    pub audit: f64,
    /// Whether a failing result (non-zero exit) may prove itself and
    /// replay. Off by default: a failure that repeats identically is as
    /// likely the box (load, a hang to go test's timeout) as the code.
    pub replay_failures: bool,
}

impl Default for Policy {
    fn default() -> Policy {
        Policy {
            mode: Mode::Observe,
            proofs: DEFAULT_PROOFS,
            audit: DEFAULT_AUDIT,
            replay_failures: false,
        }
    }
}

/// Text in a step's stdout or stderr that says something was killed or
/// timed out rather than finished: go test's own timeout panic and its
/// kill of a test binary that ignored it, a child reported dead by signal,
/// the Go runtime out of memory. A result carrying one is never provable,
/// whatever its exit code (`go test ./... || true` exits 0): what it shows
/// is the box's condition, not the code's.
pub const KILL_MARKERS: &[&str] = &[
    "panic: test timed out after",
    "*** Test killed",
    "signal: killed",
    "signal: terminated",
    "signal: interrupt",
    "fatal error: out of memory",
];

impl Policy {
    pub fn validate(&self) -> Result<(), TrustError> {
        if self.proofs == 0 {
            return Err(TrustError::BadPolicy(
                "--proofs must be at least 1: a result seen zero times cannot replay".into(),
            ));
        }
        if self.proofs > MAX_PROOFS {
            return Err(TrustError::BadPolicy(format!(
                "--proofs {} is more than {MAX_PROOFS}",
                self.proofs
            )));
        }
        if !(0.0..=1.0).contains(&self.audit) || self.audit.is_nan() {
            return Err(TrustError::BadPolicy(format!(
                "--audit {} is not a fraction in 0..=1",
                self.audit
            )));
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TrustError {
    #[error(transparent)]
    Ledger(#[from] LedgerError),
    #[error(transparent)]
    Cas(#[from] CasError),
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error(transparent)]
    Canon(#[from] CanonError),
    #[error("read blob: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    BadPolicy(String),
}

/// Why a step executes although the ledger was consulted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reason {
    /// No manifest of the key matched the tree, or none had a result.
    Miss,
    /// A manifest matched, but its result can never prove itself (see
    /// `unprovable`): a failure, or output carrying a kill marker.
    Unprovable { why: String },
    /// A manifest matched, but its result is not yet reproduced often enough.
    Proving { streak: u32, needed: u32 },
    /// The key is quarantined after a divergence; `streak` of `needed`
    /// identical executions toward re-proof.
    Quarantined {
        since: String,
        why: String,
        streak: u32,
        needed: u32,
    },
    /// Replay mode: this would-be hit (the `nth` for the key) is audited.
    Audit { nth: u64 },
    /// Observe mode: this would have replayed.
    WouldReplay,
}

impl Reason {
    /// Does this execution count as a would-be hit?
    fn eligible(&self) -> bool {
        matches!(self, Reason::Audit { .. } | Reason::WouldReplay)
    }

    fn phase(&self) -> &'static str {
        match self {
            Reason::Audit { .. } => "audit",
            Reason::WouldReplay => "observe",
            _ => "proof",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    Replay,
    Execute(Reason),
}

/// What `Trust::record_replay` concluded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Replayed {
    /// Recorded; the receipt's trust facts.
    Replayed(Facts),
    /// Decided otherwise inside the transaction: execute, for this reason.
    Execute(Reason),
}

/// Identical executions a key's manifests need: the policy's count plus one
/// for every divergence among the key's newest `TRAILING_AUDITS` audits, at
/// most `MAX_EXTRA_PROOFS` more. A key that keeps diverging needs longer
/// runs of identical results; one that stops diverging gets back to the
/// base bar as clean audits push its divergences out of the window.
pub fn needed(policy: &Policy, recent_divergences: u64) -> u32 {
    let extra = u32::try_from(recent_divergences)
        .unwrap_or(u32::MAX)
        .min(MAX_EXTRA_PROOFS);
    policy.proofs.saturating_add(extra)
}

/// What `decide` needs to know about the result a replay would serve.
#[derive(Clone, Copy, Debug)]
pub struct HitView<'a> {
    /// The run that recorded it (the matched manifest's newest result).
    pub run_id: &'a str,
    /// Why it can never prove itself, if it cannot (`unprovable`).
    pub unprovable: Option<&'a str>,
}

/// The replay decision from the policy and the stored facts. Pure. The
/// hit's proof streak counts only if the hit's run is the one the streak
/// ends with: the streak vouches for the result that run recorded, and for
/// no other (a ledger where they disagree, e.g. a result that was never
/// stored after its proof, is treated as unproven). An unprovable result
/// never replays, whatever its streak.
pub fn decide(
    policy: &Policy,
    hit: Option<HitView>,
    trust: Option<&TrustRow>,
    proof: Option<&ProofRow>,
    recent_divergences: u64,
) -> Decision {
    let Some(hit) = hit else {
        return Decision::Execute(Reason::Miss);
    };
    let streak = proof
        .filter(|p| p.last_run == hit.run_id)
        .map_or(0, |p| p.streak);
    let needed = needed(policy, recent_divergences);
    if let Some(t) = trust
        && t.state == TrustState::Quarantined
    {
        return Decision::Execute(Reason::Quarantined {
            since: t.quarantined_at.clone().unwrap_or_default(),
            why: t.quarantine_reason.clone().unwrap_or_default(),
            streak,
            needed,
        });
    }
    if let Some(why) = hit.unprovable {
        return Decision::Execute(Reason::Unprovable { why: why.into() });
    }
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
}

/// Whether the next would-be hit of a key (its `eligible_before + 1`th) is
/// audited: deterministic, every `1/rate`th one, so a given rate yields the
/// same schedule on every ledger and tests can rely on it. A step cannot
/// game it: it never sees the count.
pub fn audit_due(eligible_before: u64, rate: f64) -> bool {
    let rate = rate.clamp(0.0, 1.0);
    if rate <= 0.0 {
        return false;
    }
    let k = eligible_before + 1;
    let f = |n: u64| ((n as f64) * rate + 1e-9).floor();
    f(k) > f(k - 1)
}

// ----- comparing results -----

/// Erases what two faithful executions of `go test` print differently:
/// per-package durations (`ok  \tpkg\t0.412s`, `FAIL\tpkg\t1.2s`, or
/// `(cached)` where go test served the result from its own cache, with any
/// fields after it, such as coverage, kept) and
/// per-test durations (`--- PASS: TestX (0.00s)`), and the number of the
/// go command's temporary build dir in a `go env` `GOGCCFLAGS=` line
/// (setup-go prints `go env` on every run). Nothing else: exit codes,
/// package and test names, failure output and counts all stay. A line of
/// any other shape is kept byte for byte.
pub fn normalize(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    for line in bytes.split_inclusive(|b| *b == b'\n') {
        let (body, nl) = match line.strip_suffix(b"\n") {
            Some(b) => (b, true),
            None => (line, false),
        };
        out.extend_from_slice(&normalize_line(body));
        if nl {
            out.push(b'\n');
        }
    }
    out
}

/// `s` with the digits after every `/tmp/go-build` replaced by `<n>`.
fn mask_go_build_dirs(s: &str) -> String {
    const DIR: &str = "/tmp/go-build";
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find(DIR) {
        out.push_str(&rest[..i + DIR.len()]);
        rest = &rest[i + DIR.len()..];
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0 {
            out.push_str("<n>");
            rest = &rest[digits..];
        }
    }
    out.push_str(rest);
    out
}

fn normalize_line(line: &[u8]) -> Vec<u8> {
    let Ok(s) = std::str::from_utf8(line) else {
        return line.to_vec();
    };
    // `go env`'s GOGCCFLAGS names the go command's temporary build dir
    // (`-ffile-prefix-map=/tmp/go-build2595014798=/tmp/go-build`), random
    // per invocation; setup-go prints `go env` on every run.
    if let Some(rest) = s.strip_prefix("GOGCCFLAGS=") {
        return format!("GOGCCFLAGS={}", mask_go_build_dirs(rest)).into_bytes();
    }
    // A package line: `ok  \tpkg\t0.412s`, or `(cached)` where go test
    // served the result from its cache (a warm step under Quod's guard,
    // quod_key::gotest), either one possibly followed by more fields
    // (`\tcoverage: 81.2% of statements`).
    if s.starts_with("ok  \t") || s.starts_with("FAIL\t") {
        let mut fields: Vec<&str> = s.split('\t').collect();
        if fields.len() >= 3 && (is_duration(fields[2]) || fields[2] == "(cached)") {
            fields[2] = "<dur>";
            return fields.join("\t").into_bytes();
        }
    }
    let t = s.trim_start();
    if ["--- PASS: ", "--- FAIL: ", "--- SKIP: ", "--- BENCH: "]
        .iter()
        .any(|p| t.starts_with(p))
        && let Some(rest) = s.strip_suffix(')')
        && let Some(open) = rest.rfind(" (")
        && is_duration(&rest[open + 2..])
    {
        return format!("{}(<dur>)", &s[..open + 1]).into_bytes();
    }
    line.to_vec()
}

/// `0.412s`, `600.00s`: digits with an optional point, then `s`.
fn is_duration(t: &str) -> bool {
    t.strip_suffix('s').is_some_and(|n| {
        !n.is_empty()
            && n.bytes().all(|b| b.is_ascii_digit() || b == b'.')
            && n.parse::<f64>().is_ok()
    })
}

/// The output entries two executions are compared on: everything except
/// what lies under `ignore` (the scratch directories Quod gives the
/// step: the toolchain's caches and HOME, which it
/// manages and which carry timestamps).
pub fn comparable_outputs<'a>(outputs: &'a [Entry], ignore: &[String]) -> Vec<&'a Entry> {
    outputs
        .iter()
        .filter(|e| {
            !ignore
                .iter()
                .any(|pre| e.p == *pre || e.p.starts_with(&format!("{pre}/")))
        })
        .collect()
}

fn read_blob(cas: &Cas, h: &Hash) -> Result<Vec<u8>, TrustError> {
    let mut buf = Vec::new();
    cas.open_blob(h)?.read_to_end(&mut buf)?;
    Ok(buf)
}

/// What a result is compared on, hashed: exit code, normalized stdout and
/// stderr, comparable outputs, and every write outside the workspace
/// (`root_outputs`, rootfs mode: compared exactly, none left out). Two
/// results with equal fingerprints are identical for the trust layer.
pub fn fingerprint(r: &StepResult, cas: &Cas, ignore: &[String]) -> Result<Hash, TrustError> {
    let out = Hash::of(&normalize(&read_blob(cas, &r.stdout)?));
    let err = Hash::of(&normalize(&read_blob(cas, &r.stderr)?));
    let outputs =
        Hash::of(quod_cas::canonical_json(&comparable_outputs(&r.outputs, ignore))?.as_bytes());
    let root = Hash::of(quod_cas::canonical_json(&r.root_outputs)?.as_bytes());
    let mut buf = Vec::with_capacity(256);
    // v2: command files hashed in, GOGCCFLAGS' build dir normalized. v3:
    // the rename to Quod (the tag only; the rules are v2's). v4: a package
    // line's `(cached)` reads as a duration, and its duration is erased
    // when more fields follow it.
    buf.extend_from_slice(b"quod result fingerprint v4\0");
    buf.extend_from_slice(&i64::from(r.exit_code).to_le_bytes());
    for h in [out, err, outputs, root] {
        buf.extend_from_slice(h.hex().as_bytes());
    }
    // What a runner-hooks step appended to its command files (GITHUB_ENV,
    // GITHUB_OUTPUT, ...), which its replay appends again: part of the
    // result. Absent (and so not hashed) for every CLI result, whose
    // fingerprint is unchanged.
    if !r.commands.is_empty() {
        buf.extend_from_slice(b"commands\0");
        buf.extend_from_slice(
            Hash::of(quod_cas::canonical_json(&r.commands)?.as_bytes())
                .hex()
                .as_bytes(),
        );
    }
    Ok(Hash::of(&buf))
}

/// Why `r` can never prove itself under `policy`, if it cannot: a failure
/// (unless the policy replays failures), or stdout or stderr carrying a
/// `KILL_MARKERS` entry.
pub fn unprovable(
    policy: &Policy,
    r: &StepResult,
    cas: &Cas,
) -> Result<Option<String>, TrustError> {
    for (name, h) in [("stdout", &r.stdout), ("stderr", &r.stderr)] {
        let bytes = read_blob(cas, h)?;
        if let Some(m) = KILL_MARKERS
            .iter()
            .find(|m| bytes.windows(m.len()).any(|w| w == m.as_bytes()))
        {
            return Ok(Some(format!(
                "its {name} says {m:?}: killed or timed out, not finished"
            )));
        }
    }
    if r.exit_code != 0 && !policy.replay_failures {
        return Ok(Some(format!(
            "exit code {}: failures never replay unless --replay-failures",
            r.exit_code
        )));
    }
    Ok(None)
}

/// How two results of the same manifest differ. Empty means identical.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Divergence {
    pub items: Vec<String>,
}

impl Divergence {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Adds every item of `other` not already named.
    fn merge(&mut self, other: Divergence) {
        for i in other.items {
            if !self.items.contains(&i) {
                self.items.push(i);
            }
        }
    }

    fn push(&mut self, item: String) {
        self.merge(Divergence { items: vec![item] });
    }
}

impl fmt::Display for Divergence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.items.join("; "))
    }
}

/// Compares `actual` with `expected` and names every difference.
pub fn compare(
    expected: &StepResult,
    actual: &StepResult,
    cas: &Cas,
    ignore: &[String],
) -> Result<Divergence, TrustError> {
    let mut d = Divergence::default();
    if expected.exit_code != actual.exit_code {
        d.items.push(format!(
            "exit code {} -> {}",
            expected.exit_code, actual.exit_code
        ));
    }
    for (name, e, a) in [
        ("stdout", expected.stdout, actual.stdout),
        ("stderr", expected.stderr, actual.stderr),
    ] {
        if e == a {
            continue;
        }
        let (x, y) = (
            normalize(&read_blob(cas, &e)?),
            normalize(&read_blob(cas, &a)?),
        );
        if x != y {
            d.items.push(first_difference(name, &x, &y));
        }
    }
    let (eo, ao) = (
        comparable_outputs(&expected.outputs, ignore),
        comparable_outputs(&actual.outputs, ignore),
    );
    if eo != ao {
        d.items.push(output_difference("outputs", &eo, &ao));
    }
    if expected.root_outputs != actual.root_outputs {
        let (er, ar): (Vec<&Entry>, Vec<&Entry>) = (
            expected.root_outputs.iter().collect(),
            actual.root_outputs.iter().collect(),
        );
        d.items
            .push(output_difference("writes outside the workspace", &er, &ar));
    }
    if expected.commands != actual.commands {
        let names = |r: &StepResult| -> BTreeMap<String, Hash> {
            r.commands.iter().map(|c| (c.name.clone(), c.h)).collect()
        };
        let (e, a) = (names(expected), names(actual));
        let differ: Vec<&str> = e
            .keys()
            .chain(a.keys())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .filter(|n| e.get(*n) != a.get(*n))
            .map(String::as_str)
            .collect();
        d.items.push(format!(
            "what it wrote to the runner's command files differs: {}",
            differ.join(", ")
        ));
    }
    Ok(d)
}

/// The lines of `t`, without a final empty one after a trailing newline.
fn lines(t: &[u8]) -> Vec<&[u8]> {
    let t = t.strip_suffix(b"\n").unwrap_or(t);
    if t.is_empty() {
        Vec::new()
    } else {
        t.split(|c| *c == b'\n').collect()
    }
}

fn first_difference(name: &str, a: &[u8], b: &[u8]) -> String {
    let (la, lb) = (lines(a), lines(b));
    let show = |l: Option<&&[u8]>| match l {
        None => "<end of output>".to_string(),
        Some(l) => {
            let s = String::from_utf8_lossy(l);
            let mut t: String = s.chars().take(80).collect();
            if t.len() < s.len() {
                t.push_str("...");
            }
            t
        }
    };
    let i = la
        .iter()
        .zip(lb.iter())
        .position(|(x, y)| x != y)
        .unwrap_or(la.len().min(lb.len()));
    format!(
        "{name} differs from line {}: {:?} vs {:?}",
        i + 1,
        show(la.get(i)),
        show(lb.get(i))
    )
}

/// Whether a hit's manifest `a` and the manifest `b` that the execution it
/// triggered recorded (on a tree `a` matches) match exactly the same trees:
/// every entry one records and the other does not is an `a` (absent) entry
/// whose parent directory's listing the other records. On that tree the
/// name is absent, so the recorded listing lacks it, and any tree matching
/// that listing lacks it too: the absent entry tells no tree apart. This is
/// the variation warm state produces by design (with a full build cache
/// the assembler's include probes beside `.s` files do not happen; their
/// directories are listed either way, see W4). Any other difference means
/// one execution read inputs the other did not, on the same tree: the
/// manifest under-describes what the step may read. `Err` names the first
/// such path.
pub fn equivalent(a: &Manifest, b: &Manifest) -> Result<(), String> {
    // Vec: `Manifest::from_log` keeps two entries for one path when a lookup
    // contradicted the tree (such a manifest can never match anything).
    fn by_path(m: &Manifest) -> BTreeMap<&str, Vec<&Entry>> {
        let mut out: BTreeMap<&str, Vec<&Entry>> = BTreeMap::new();
        for e in &m.entries {
            out.entry(e.p.as_str()).or_default().push(e);
        }
        out
    }
    let (ma, mb) = (by_path(a), by_path(b));
    let covered = |es: &[&Entry], other: &BTreeMap<&str, Vec<&Entry>>| {
        es.iter().all(|e| {
            e.k == Kind::Absent
                && other
                    .get(parent_and_name(&e.p).0)
                    .is_some_and(|d| d.iter().any(|d| d.k == Kind::Dir))
        })
    };
    for p in ma
        .keys()
        .chain(mb.keys())
        .collect::<std::collections::BTreeSet<_>>()
    {
        match (ma.get(p), mb.get(p)) {
            (Some(x), Some(y)) if x != y => return Err(format!("{p} recorded differently")),
            (Some(x), None) if !covered(x, &mb) => {
                return Err(format!("{p} read by the earlier execution only"));
            }
            (None, Some(y)) if !covered(y, &ma) => {
                return Err(format!("{p} read by this execution only"));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Whether two results are the same bytes on everything compared (no
/// normalization was needed).
fn same_bytes(a: &StepResult, b: &StepResult, ignore: &[String]) -> bool {
    a.exit_code == b.exit_code
        && a.stdout == b.stdout
        && a.stderr == b.stderr
        && comparable_outputs(&a.outputs, ignore) == comparable_outputs(&b.outputs, ignore)
        && a.root_outputs == b.root_outputs
        && a.commands == b.commands
}

fn output_difference(what: &str, e: &[&Entry], a: &[&Entry]) -> String {
    let em: BTreeMap<&str, &Entry> = e.iter().map(|x| (x.p.as_str(), *x)).collect();
    let am: BTreeMap<&str, &Entry> = a.iter().map(|x| (x.p.as_str(), *x)).collect();
    let mut diffs = Vec::new();
    for p in em
        .keys()
        .chain(am.keys())
        .collect::<std::collections::BTreeSet<_>>()
    {
        match (em.get(p), am.get(p)) {
            (Some(x), Some(y)) if x != y => diffs.push(format!("{p} changed")),
            (Some(_), None) => diffs.push(format!("{p} missing")),
            (None, Some(_)) => diffs.push(format!("{p} new")),
            _ => {}
        }
    }
    let shown: Vec<&str> = diffs.iter().take(3).map(String::as_str).collect();
    format!(
        "{what} differ at {} path(s): {}{}",
        diffs.len(),
        shown.join(", "),
        if diffs.len() > 3 { ", ..." } else { "" }
    )
}

// ----- recording -----

/// The step being decided on, as the trust layer sees it.
pub struct Step<'a> {
    pub key: Hash,
    /// The script text, kept with the trust row for `status`.
    pub cmd: &'a str,
    /// Output prefixes left out of comparisons (the step's scratch dirs).
    pub ignore: &'a [String],
}

/// Trust facts about a replayed or replayable result, for receipts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Facts {
    pub trust: TrustRow,
    pub streak: u32,
    pub needed: u32,
}

impl Facts {
    /// `  trust: proven by 3 identical executions, audited 17 times, 0 divergences`
    pub fn receipt_line(&self) -> String {
        format!(
            "trust: proven by {} identical execution{}, audited {} time{}, {} divergence{}",
            self.streak,
            plural(u64::from(self.streak)),
            self.trust.audits,
            plural(self.trust.audits),
            self.trust.divergences,
            plural(self.trust.divergences)
        )
    }
}

fn plural(n: u64) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// `2026-11-14T09:12:03.007Z` -> `2026-11-14 09:12 UTC`.
fn human_time(rfc3339: &str) -> String {
    match (rfc3339.get(..10), rfc3339.get(11..16)) {
        (Some(d), Some(t)) => format!("{d} {t} UTC"),
        _ => rfc3339.to_string(),
    }
}

fn fmt_duration(d: Duration) -> String {
    if d.as_secs() >= 60 {
        let s = d.as_secs();
        format!("{}m{:02}s", s / 60, s % 60)
    } else {
        format!("{:.1}s", d.as_secs_f64())
    }
}

/// What recording an execution concluded, for the banner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    pub reason: Reason,
    /// The run this execution was compared with, if any.
    pub compared_with: Option<String>,
    /// How it differed, if it did (the key is now quarantined).
    pub divergence: Option<Divergence>,
    /// Whether the compared bytes were identical before normalization.
    pub byte_identical: bool,
    pub streak: u32,
    pub needed: u32,
    pub trust: TrustRow,
    /// Observe mode, would have replayed: the recorded duration.
    pub would_have_saved: Option<Duration>,
    /// Why this execution's result can never prove itself, if it cannot.
    pub unprovable: Option<String>,
}

impl Verdict {
    /// Whether the manifest this execution recorded may replay from now on.
    pub fn replayable(&self) -> bool {
        self.streak >= self.needed
            && self.trust.state != TrustState::Quarantined
            && self.unprovable.is_none()
    }

    /// `replayed`, `miss`, ... for the run row.
    pub fn outcome(&self) -> &'static str {
        if self.divergence.is_some() {
            return "diverged";
        }
        match self.reason {
            Reason::Miss => "miss",
            Reason::Unprovable { .. } => "unprovable",
            Reason::Proving { .. } | Reason::Quarantined { .. } => "proving",
            Reason::Audit { .. } => "audit",
            Reason::WouldReplay => "would-replay",
        }
    }

    /// What to append to the EXECUTED headline: why a would-be hit executed
    /// (an audit, or observe mode), whatever the comparison then found.
    pub fn headline(&self) -> String {
        match (&self.reason, self.would_have_saved) {
            (Reason::WouldReplay, Some(d)) => format!(
                "  (observe mode: would have replayed, {} not saved)",
                fmt_duration(d)
            ),
            (Reason::WouldReplay, None) => "  (observe mode: would have replayed)".into(),
            (Reason::Audit { nth }, _) => format!("  (audit: would-be hit {nth} re-executed)"),
            _ => String::new(),
        }
    }

    /// The trust lines of the banner, without indentation; a divergence
    /// comes as its own loud block.
    pub fn lines(&self) -> Vec<String> {
        let t = &self.trust;
        let audited = format!(
            "audited {} time{}, {} divergence{}",
            t.audits,
            plural(t.audits),
            t.divergences,
            plural(t.divergences)
        );
        if let Some(d) = &self.divergence {
            let mut v = vec![format!(
                "QUOD  DIVERGENCE  step key quarantined: this execution differs from run {} on identical inputs",
                self.compared_with
                    .as_deref()
                    .map(|r| &r[..r.len().min(8)])
                    .unwrap_or("?")
            )];
            v.extend(d.items.iter().map(|i| format!("    {i}")));
            v.push(format!(
                "  this key executes until {} identical executions prove it again; {audited}",
                self.needed
            ));
            return v;
        }
        let identical = match &self.compared_with {
            Some(r) => format!(
                ", {} run {}",
                if self.byte_identical {
                    "byte-identical to"
                } else {
                    "identical (up to go test durations) to"
                },
                &r[..r.len().min(8)]
            ),
            None => String::new(),
        };
        if let Some(why) = &self.unprovable
            && !matches!(self.reason, Reason::Quarantined { .. })
        {
            return vec![format!(
                "trust: not provable ({why}); it executes every time{identical}"
            )];
        }
        match &self.reason {
            Reason::Miss if self.compared_with.is_none() => vec![format!(
                "trust: proof {} of {}; replay needs {} identical execution{}",
                self.streak,
                self.needed,
                self.needed,
                plural(u64::from(self.needed))
            )],
            Reason::Miss | Reason::Proving { .. } | Reason::Unprovable { .. } => vec![format!(
                "trust: proof {} of {}{identical}{}",
                self.streak,
                self.needed,
                if self.replayable() {
                    "; replayable from the next matching run"
                } else {
                    ""
                }
            )],
            Reason::Quarantined { since, why, .. } => vec![format!(
                "trust: quarantined since {} ({}); re-proof {} of {}{identical}{}",
                human_time(since),
                why,
                self.streak,
                self.needed,
                if self.replayable() {
                    "; proven again, replayable from the next matching run"
                } else {
                    ""
                }
            )],
            Reason::Audit { .. } => vec![format!("trust: audit passed{identical}; {audited}")],
            Reason::WouldReplay => vec![format!(
                "trust: {audited}; `quod run --mode replay` replays this"
            )],
        }
    }
}

/// The trust layer over a ledger and a CAS.
pub struct Trust<'a> {
    ledger: &'a Ledger,
    cas: &'a Cas,
    pub policy: Policy,
}

impl<'a> Trust<'a> {
    pub fn new(ledger: &'a Ledger, cas: &'a Cas, policy: Policy) -> Result<Trust<'a>, TrustError> {
        policy.validate()?;
        Ok(Trust {
            ledger,
            cas,
            policy,
        })
    }

    /// Replay or execute, given what the lookup found.
    pub fn decide(
        &self,
        key: &Hash,
        hit: Option<&(Manifest, StepResult)>,
    ) -> Result<Decision, TrustError> {
        let trust = self.ledger.trust(key)?;
        let (proof, why) = match hit {
            Some((m, r)) => (
                self.ledger.proof(key, &manifest_id(m)?)?,
                unprovable(&self.policy, r, self.cas)?,
            ),
            None => (None, None),
        };
        Ok(decide(
            &self.policy,
            hit.map(|(_, r)| HitView {
                run_id: &r.run_id,
                unprovable: why.as_deref(),
            }),
            trust.as_ref(),
            proof.as_ref(),
            self.ledger.recent_divergences(key, TRAILING_AUDITS)?,
        ))
    }

    /// The facts about a key and one of its manifests.
    pub fn facts(&self, key: &Hash, manifest: &Hash) -> Result<Facts, TrustError> {
        let trust = self
            .ledger
            .trust(key)?
            .unwrap_or_else(|| TrustRow::new(*key, None));
        let streak = self.ledger.proof(key, manifest)?.map_or(0, |p| p.streak);
        let needed = needed(
            &self.policy,
            self.ledger.recent_divergences(key, TRAILING_AUDITS)?,
        );
        Ok(Facts {
            trust,
            streak,
            needed,
        })
    }

    /// Records a replay, deciding again inside the transaction: between
    /// `decide` and now another run may have quarantined the key or taken
    /// the audit slot this hit was going to skip. `Execute` means the
    /// caller must execute after all, for that reason.
    pub fn record_replay(
        &self,
        step: &Step,
        hit: &(Manifest, StepResult),
    ) -> Result<Replayed, TrustError> {
        let (m, r) = hit;
        let mid = manifest_id(m)?;
        let why = unprovable(&self.policy, r, self.cas)?;
        self.ledger.atomically(|l| {
            let trust = l.trust(&step.key)?;
            let decision = decide(
                &self.policy,
                Some(HitView {
                    run_id: &r.run_id,
                    unprovable: why.as_deref(),
                }),
                trust.as_ref(),
                l.proof(&step.key, &mid)?.as_ref(),
                l.recent_divergences(&step.key, TRAILING_AUDITS)?,
            );
            if let Decision::Execute(reason) = decision {
                return Ok(Replayed::Execute(reason));
            }
            let mut t = trust.unwrap_or_else(|| TrustRow::new(step.key, Some(step.cmd)));
            t.eligible += 1;
            t.replays += 1;
            l.put_trust(&t)?;
            Ok(Replayed::Replayed(self.facts(&step.key, &mid)?))
        })
    }

    fn result_of_run(&self, run_id: &str, key: &Hash) -> Result<Option<StepResult>, TrustError> {
        Ok(self
            .ledger
            .results_for_run(run_id)?
            .into_iter()
            .find(|r| r.step_key == *key))
    }

    /// Records an execution that finished with a result. Call it before
    /// `put_result`, in the same `Ledger::atomically` if the caller wants
    /// both or neither (it joins the caller's transaction; on its own it is
    /// one transaction).
    ///
    /// The execution is compared with what a replay would have served (the
    /// hit's result) and with the last execution of the manifest it read,
    /// if that is another run. Any difference in result, or a read set that
    /// is not `equivalent` to the hit's, is a divergence: the key is
    /// quarantined and every streak of it restarts. Otherwise the streak of
    /// this manifest advances if its last execution had the same result,
    /// and starts at 1 if not seen before. Every comparison is an audit row.
    pub fn record_execution(
        &self,
        step: &Step,
        run_id: &str,
        executed: &Manifest,
        result: &StepResult,
        hit: Option<&(Manifest, StepResult)>,
        reason: &Reason,
    ) -> Result<Verdict, TrustError> {
        let manifest = manifest_id(executed)?;
        let fp = fingerprint(result, self.cas, step.ignore)?;
        let hit_manifest = hit.map(|(m, _)| manifest_id(m)).transpose()?;
        let why_unprovable = unprovable(&self.policy, result, self.cas)?;
        self.ledger.atomically(|l| {
            let key = step.key;
            let mut trust = l
                .trust(&key)?
                .unwrap_or_else(|| TrustRow::new(key, Some(step.cmd)));
            if trust.cmd.is_none() {
                trust.cmd = Some(step.cmd.to_string());
            }
            if reason.eligible() {
                trust.eligible += 1;
            }
            let prior = l.proof(&key, &manifest)?;

            let mut divergence = Divergence::default();
            let mut compared_with: Option<String> = None;
            let mut byte_identical = true;

            // What a replay would have served. The tree matched its
            // manifest; the step must read the same inputs (up to absent
            // probes a recorded listing already pins) and produce the same
            // result.
            if let (Some((hm, hr)), Some(hid)) = (hit, hit_manifest) {
                if hid != manifest
                    && let Err(at) = equivalent(hm, executed)
                {
                    divergence.push(format!(
                        "inputs differed: the tree matched manifest b3:{} but this execution read manifest b3:{} ({at})",
                        hid.short(),
                        manifest.short()
                    ));
                }
                divergence.merge(compare(hr, result, self.cas, step.ignore)?);
                byte_identical &= same_bytes(hr, result, step.ignore);
                compared_with = Some(hr.run_id.clone());
            }
            // The last execution of the manifest this run read, when that
            // is another run than the hit's (a manifest read on another
            // tree, or a ledger written before the trust layer). Its
            // fingerprint is recomputed under the current rules (normalize,
            // the ignored outputs, HOME_VOLATILE): a stored fingerprint was
            // computed under whatever rules the run that wrote it had, so
            // comparing it after the rules changed would read a rules change
            // as a divergence. Only a result that is gone (purged) leaves the
            // stored one to compare, and then a mismatch restarts the streak,
            // never quarantines.
            let mut streak = 1;
            if let Some(p) = &prior {
                let prev_fp = if compared_with.as_deref() == Some(p.last_run.as_str()) {
                    let (_, hr) = hit.expect("compared_with is the hit's run");
                    Some(fingerprint(hr, self.cas, step.ignore)?)
                } else {
                    match self.result_of_run(&p.last_run, &key)? {
                        Some(prev) => {
                            compared_with.get_or_insert_with(|| p.last_run.clone());
                            divergence.merge(compare(&prev, result, self.cas, step.ignore)?);
                            byte_identical &= same_bytes(&prev, result, step.ignore);
                            Some(fingerprint(&prev, self.cas, step.ignore)?)
                        }
                        None => None,
                    }
                };
                let same = match prev_fp {
                    Some(pf) => {
                        if pf != fp && divergence.is_empty() {
                            // compare() found nothing to name, yet the
                            // fingerprints differ under the same rules: say
                            // so rather than call them identical.
                            divergence.push(
                                "result fingerprint differs from the previous execution".into(),
                            );
                        }
                        pf == fp
                    }
                    None => {
                        // Equal stored fingerprints still say the results
                        // agree (on the old rules and the new alike); that
                        // counts as a comparison. A mismatch says nothing.
                        let same = p.fingerprint == fp;
                        byte_identical = false;
                        if same {
                            compared_with.get_or_insert_with(|| p.last_run.clone());
                        }
                        same
                    }
                };
                if same {
                    streak = p.streak + 1;
                }
            }
            let compared = compared_with.is_some();
            byte_identical &= compared;
            if compared {
                trust.audits += 1;
            }

            if !divergence.is_empty() {
                trust.divergences += 1;
                trust.state = TrustState::Quarantined;
                trust.quarantined_at = Some(now_rfc3339());
                trust.quarantine_reason = Some(truncate(&divergence.to_string(), 300));
                // Every manifest of the key starts proving again; this one's
                // new result is the first of its new streak.
                l.reset_proofs(&key)?;
                streak = 1;
                tracing::warn!(target: "quod", step_key = %key, manifest_id = %manifest, divergence = %divergence, "divergence: step key quarantined");
            }
            if why_unprovable.is_some() {
                // Compared like any result (a later different one is still
                // a divergence), but it never counts toward a replay.
                streak = 0;
            }
            l.put_proof(&ProofRow {
                step_key: key,
                manifest_id: manifest,
                streak,
                fingerprint: fp,
                last_run: run_id.to_string(),
                updated_at: String::new(),
            })?;
            // This execution's audit row is written below: count it now.
            let recent = l.recent_divergences(&key, TRAILING_AUDITS.saturating_sub(1))?
                + u64::from(!divergence.is_empty());
            let needed = needed(&self.policy, recent);
            if divergence.is_empty()
                && why_unprovable.is_none()
                && streak >= needed
                && trust.state != TrustState::Proven
            {
                trust.state = TrustState::Proven;
                trust.quarantined_at = None;
                trust.quarantine_reason = None;
            }
            l.put_trust(&trust)?;
            if compared {
                l.record_audit(&AuditRow {
                    id: 0,
                    step_key: key,
                    manifest_id: manifest,
                    at: String::new(),
                    phase: reason.phase().into(),
                    diverged: !divergence.is_empty(),
                    expected_run: compared_with.clone(),
                    actual_run: Some(run_id.to_string()),
                    detail: Some(if divergence.is_empty() {
                        if byte_identical {
                            "byte-identical".to_string()
                        } else {
                            "identical up to go test durations".to_string()
                        }
                    } else {
                        truncate(&divergence.to_string(), 2000)
                    }),
                })?;
            }
            Ok(Verdict {
                reason: reason.clone(),
                compared_with,
                divergence: (!divergence.is_empty()).then_some(divergence),
                byte_identical,
                streak,
                needed,
                trust,
                would_have_saved: match reason {
                    Reason::WouldReplay => hit.map(|(_, r)| Duration::from_millis(r.duration_ms)),
                    _ => None,
                },
                unprovable: why_unprovable.clone(),
            })
        })
    }

    /// Records an execution of a hit that produced no result to compare,
    /// yet behaved otherwise than the run a replay would serve: the runner
    /// hooks run a keyed step offline, and on the same inputs it failed,
    /// reached for the network, or wrote where the recorded run did not
    /// (`why` says how). That is a divergence like any other: the key is
    /// quarantined, every manifest of it proves again, and the audit row
    /// says what happened. A run that was the machine's (killed, out of
    /// memory) is not one: that is `record_incomplete`.
    pub fn record_divergence(
        &self,
        step: &Step,
        run_id: &str,
        hit: &(Manifest, StepResult),
        reason: &Reason,
        why: &str,
    ) -> Result<Verdict, TrustError> {
        let (m, hr) = hit;
        let manifest = manifest_id(m)?;
        self.ledger.atomically(|l| {
            let key = step.key;
            let mut trust = l
                .trust(&key)?
                .unwrap_or_else(|| TrustRow::new(key, Some(step.cmd)));
            if trust.cmd.is_none() {
                trust.cmd = Some(step.cmd.to_string());
            }
            if reason.eligible() {
                trust.eligible += 1;
            }
            let mut divergence = Divergence::default();
            divergence.push(format!("this execution {why}"));
            trust.audits += 1;
            trust.divergences += 1;
            trust.state = TrustState::Quarantined;
            trust.quarantined_at = Some(now_rfc3339());
            trust.quarantine_reason = Some(truncate(&divergence.to_string(), 300));
            l.reset_proofs(&key)?;
            l.put_trust(&trust)?;
            l.record_audit(&AuditRow {
                id: 0,
                step_key: key,
                manifest_id: manifest,
                at: String::new(),
                phase: reason.phase().into(),
                diverged: true,
                expected_run: Some(hr.run_id.clone()),
                actual_run: Some(run_id.to_string()),
                detail: Some(truncate(&divergence.to_string(), 2000)),
            })?;
            let needed = needed(&self.policy, l.recent_divergences(&key, TRAILING_AUDITS)?);
            tracing::warn!(target: "quod", step_key = %key, manifest_id = %manifest, divergence = %divergence, "divergence without a result: step key quarantined");
            Ok(Verdict {
                reason: reason.clone(),
                compared_with: Some(hr.run_id.clone()),
                divergence: Some(divergence),
                byte_identical: false,
                streak: 0,
                needed,
                trust,
                would_have_saved: None,
                unprovable: None,
            })
        })
    }

    /// Records a step that did not finish (`what`: "timed out after 30m00s").
    /// Nothing is memoized, and nothing is learned about the step's result:
    /// a timeout is as likely the box (load, a slow observer) as the step,
    /// so it is not a divergence. But the result that was expected did not
    /// reproduce, so the matched manifest's streak restarts. Returns the
    /// banner line, or `None` when nothing was expected.
    pub fn record_incomplete(
        &self,
        step: &Step,
        hit: Option<&(Manifest, StepResult)>,
        what: &str,
    ) -> Result<Option<String>, TrustError> {
        let Some((m, prev)) = hit else {
            return Ok(None);
        };
        let manifest = manifest_id(m)?;
        self.ledger.atomically(|l| {
            l.reset_proof(&step.key, &manifest)?;
            let needed = needed(
                &self.policy,
                l.recent_divergences(&step.key, TRAILING_AUDITS)?,
            );
            tracing::info!(target: "quod", step_key = %step.key, manifest_id = %manifest, what, "incomplete: proof streak reset");
            Ok(Some(format!(
                "trust: the step {what}, which is not a result (run {} finished with exit code {}); manifest b3:{} proves again from 0 of {needed}",
                short_run(&prev.run_id),
                prev.exit_code,
                manifest.short()
            )))
        })
    }
}

/// The first 8 characters of a run id, as receipts show it.
fn short_run(run_id: &str) -> &str {
    &run_id[..run_id.len().min(8)]
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max).collect();
        t.push_str("...");
        t
    }
}

/// Divergences and comparisons among the newest audits, for the trailing
/// rate: `(diverged, total)`.
pub fn divergence_rate(audits: &[AuditRow]) -> (usize, usize) {
    (audits.iter().filter(|a| a.diverged).count(), audits.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use quod_cas::Tree;
    use quod_manifest::Access;

    fn trust_row(state: TrustState, eligible: u64, divergences: u64) -> TrustRow {
        TrustRow {
            state,
            eligible,
            divergences,
            quarantined_at: (state == TrustState::Quarantined)
                .then(|| "2026-10-31T10:00:00.000Z".to_string()),
            quarantine_reason: (state == TrustState::Quarantined)
                .then(|| "exit code 0 -> 1".to_string()),
            ..TrustRow::new(Hash::of(b"k"), Some("go test ./..."))
        }
    }

    fn proof(streak: u32) -> ProofRow {
        ProofRow {
            step_key: Hash::of(b"k"),
            manifest_id: Hash::of(b"m"),
            streak,
            fingerprint: Hash::of(b"fp"),
            last_run: "01RUN".into(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn decisions() {
        let replay = Policy {
            mode: Mode::Replay,
            ..Policy::default()
        };
        let observe = Policy::default();
        // `hit`: a result from run 01RUN, the run the proofs end with.
        let view = |run_id| HitView {
            run_id,
            unprovable: None,
        };
        let d = |p: &Policy, hit: bool, t: Option<&TrustRow>, pr: Option<&ProofRow>| {
            decide(
                p,
                hit.then_some(view("01RUN")),
                t,
                pr,
                t.map_or(0, |t| t.divergences),
            )
        };
        assert_eq!(
            d(&replay, false, None, None),
            Decision::Execute(Reason::Miss)
        );
        // A streak vouches only for the result of the run it ends with: a
        // hit serving another run's result is unproven, however long.
        assert_eq!(
            decide(&replay, Some(view("01OTHER")), None, Some(&proof(9)), 0),
            Decision::Execute(Reason::Proving {
                streak: 0,
                needed: 2
            })
        );
        // A hit on a manifest seen once: not proven.
        assert_eq!(
            d(&replay, true, None, Some(&proof(1))),
            Decision::Execute(Reason::Proving {
                streak: 1,
                needed: 2
            })
        );
        assert_eq!(
            d(&replay, true, None, Some(&proof(2))),
            Decision::Replay,
            "two identical executions, no audit due on the first eligible hit at 5%"
        );
        assert_eq!(
            d(&observe, true, None, Some(&proof(2))),
            Decision::Execute(Reason::WouldReplay)
        );
        // An unprovable result never replays, however long its streak.
        assert_eq!(
            decide(
                &replay,
                Some(HitView {
                    run_id: "01RUN",
                    unprovable: Some("exit code 1")
                }),
                None,
                Some(&proof(9)),
                0
            ),
            Decision::Execute(Reason::Unprovable {
                why: "exit code 1".into()
            })
        );
        // The 20th eligible hit is audited at 5%.
        let t19 = trust_row(TrustState::Proven, 19, 0);
        assert_eq!(
            d(&replay, true, Some(&t19), Some(&proof(5))),
            Decision::Execute(Reason::Audit { nth: 20 })
        );
        let t18 = trust_row(TrustState::Proven, 18, 0);
        assert_eq!(
            d(&replay, true, Some(&t18), Some(&proof(5))),
            Decision::Replay
        );
        // Quarantined: executes whatever the streak, until re-proven.
        let q = trust_row(TrustState::Quarantined, 30, 1);
        assert!(matches!(
            d(&replay, true, Some(&q), Some(&proof(9))),
            Decision::Execute(Reason::Quarantined {
                needed: 3,
                streak: 9,
                ..
            })
        ));
        // Past divergences raise the bar for every manifest of the key.
        let scarred = trust_row(TrustState::Proven, 0, 2);
        assert_eq!(
            d(&replay, true, Some(&scarred), Some(&proof(3))),
            Decision::Execute(Reason::Proving {
                streak: 3,
                needed: 4
            })
        );
        assert_eq!(
            d(&replay, true, Some(&scarred), Some(&proof(4))),
            Decision::Replay
        );
        assert!(
            Policy {
                proofs: 0,
                ..replay
            }
            .validate()
            .is_err()
        );
        assert!(
            Policy {
                audit: 1.5,
                ..replay
            }
            .validate()
            .is_err()
        );
        assert!(
            Policy {
                audit: 0.0,
                ..replay
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn audit_schedule_is_every_nth_hit() {
        let due = |rate: f64, n: u64| (0..n).filter(|k| audit_due(*k, rate)).collect::<Vec<_>>();
        assert_eq!(due(0.0, 100), Vec::<u64>::new());
        assert_eq!(due(1.0, 4), [0, 1, 2, 3]);
        assert_eq!(due(0.5, 8), [1, 3, 5, 7]);
        assert_eq!(due(0.05, 60), [19, 39, 59]);
        assert_eq!(due(0.1, 30), [9, 19, 29]);
        assert_eq!(due(0.01, 250), [99, 199]);
    }

    #[test]
    fn normalizes_only_go_test_durations_and_build_dirs() {
        let text = b"ok  \tgithub.com/cli/cli/v2/api\t0.412s\nFAIL\tgithub.com/cli/cli/v2/pkg\t600.012s\n--- PASS: TestX (0.00s)\n    --- FAIL: TestX/sub (12.5s)\n--- SKIP: TestY (0.00s)\n?   \tpkg\t[no test files]\nok  \tpkg\t(cached)\npanic: test timed out after 10m0s\nexpected 2s got 3s\nok  \tpkg\t0.5s\tcoverage: 80.0%\n";
        let want = b"ok  \tgithub.com/cli/cli/v2/api\t<dur>\nFAIL\tgithub.com/cli/cli/v2/pkg\t<dur>\n--- PASS: TestX (<dur>)\n    --- FAIL: TestX/sub (<dur>)\n--- SKIP: TestY (<dur>)\n?   \tpkg\t[no test files]\nok  \tpkg\t<dur>\npanic: test timed out after 10m0s\nexpected 2s got 3s\nok  \tpkg\t<dur>\tcoverage: 80.0%\n";
        assert_eq!(
            String::from_utf8_lossy(&normalize(text)),
            String::from_utf8_lossy(want)
        );
        assert_eq!(normalize(b"no newline at end"), b"no newline at end");
        // A result go test served from its cache reads like one it ran;
        // anything else in the third field stays.
        assert_eq!(
            normalize(b"ok  \tpkg\t(cached)\tcoverage: 1.0%\nok  \tpkg\t(stale)\nFAIL\tpkg [build failed]\n"),
            b"ok  \tpkg\t<dur>\tcoverage: 1.0%\nok  \tpkg\t(stale)\nFAIL\tpkg [build failed]\n"
        );
        // go env's GOGCCFLAGS: only the build dir's number goes.
        assert_eq!(
            normalize(b"GOGCCFLAGS='-fPIC -ffile-prefix-map=/tmp/go-build2595014798=/tmp/go-build -gno'\n"),
            b"GOGCCFLAGS='-fPIC -ffile-prefix-map=/tmp/go-build<n>=/tmp/go-build -gno'\n"
        );
        assert_eq!(
            normalize(b"CC=/tmp/go-build123\n"),
            b"CC=/tmp/go-build123\n"
        );
        assert_eq!(normalize(b"\xff\xfe binary\n"), b"\xff\xfe binary\n");
        assert!(
            !is_duration("10m0s") && !is_duration("s") && is_duration("0.00s") && is_duration("7s")
        );
    }

    struct World {
        _tmp: tempfile::TempDir,
        cas: Cas,
        ledger: Ledger,
        key: Hash,
        manifest: Manifest,
        mid: Hash,
        runs: u32,
    }

    impl World {
        fn new() -> World {
            let tmp = tempfile::tempdir().unwrap();
            let cas = Cas::open(tmp.path().join("cas")).unwrap();
            let ledger = Ledger::open_in_memory().unwrap();
            let key = Hash::of(b"go test ./...");
            let tree = Tree::from_leaves([Entry::file("go.mod", Hash::of(b"m"), 0o644)]);
            let manifest = Manifest::from_log(
                key,
                vec!["/workspace".into()],
                &tree,
                [Access {
                    path: "go.mod".into(),
                    kind: Kind::File,
                }],
            )
            .unwrap();
            let mid = manifest_id(&manifest).unwrap();
            World {
                _tmp: tmp,
                cas,
                ledger,
                key,
                manifest,
                mid,
                runs: 0,
            }
        }

        fn result(&mut self, exit: i32, stdout: &[u8], outputs: Vec<Entry>) -> StepResult {
            self.runs += 1;
            World::mk(
                &self.cas, self.key, self.mid, self.runs, exit, stdout, outputs,
            )
        }

        fn mk(
            cas: &Cas,
            key: Hash,
            mid: Hash,
            n: u32,
            exit: i32,
            stdout: &[u8],
            outputs: Vec<Entry>,
        ) -> StepResult {
            StepResult {
                v: 1,
                step_key: key,
                manifest_id: mid,
                run_id: format!("01RUN{n:02}"),
                recorded_at: now_rfc3339(),
                exit_code: exit,
                stdout: cas.put(stdout).unwrap(),
                stderr: cas.put(b"").unwrap(),
                outputs,
                root_outputs: vec![],
                duration_ms: 20_000,
                commands: vec![],
            }
        }

        /// One `quod run` in replay mode: decide, then execute (if it
        /// says so) with the given outcome and record it.
        fn run(&mut self, policy: &Policy, exit: i32, stdout: &[u8]) -> Result<Verdict, Decision> {
            self.ledger.put_manifest(&self.manifest, "setup").unwrap();
            let hit = self
                .ledger
                .latest_result(&self.key, &self.mid)
                .unwrap()
                .map(|r| (self.manifest.clone(), r));
            let trust = Trust::new(&self.ledger, &self.cas, *policy).unwrap();
            let decision = trust.decide(&self.key, hit.as_ref()).unwrap();
            let reason = match decision {
                Decision::Execute(reason) => reason,
                Decision::Replay => {
                    let step = Step {
                        key: self.key,
                        cmd: "go test ./...",
                        ignore: &[],
                    };
                    match trust.record_replay(&step, hit.as_ref().unwrap()).unwrap() {
                        Replayed::Replayed(_) => return Err(Decision::Replay),
                        Replayed::Execute(reason) => reason,
                    }
                }
            };
            // Disjoint fields: `trust` holds the ledger and the CAS.
            self.runs += 1;
            let r = World::mk(
                &self.cas,
                self.key,
                self.mid,
                self.runs,
                exit,
                stdout,
                vec![],
            );
            let ignore = vec![".quod/gocache".to_string()];
            let v = trust
                .record_execution(
                    &Step {
                        key: self.key,
                        cmd: "go test ./...",
                        ignore: &ignore,
                    },
                    &r.run_id,
                    &self.manifest,
                    &r,
                    hit.as_ref(),
                    &reason,
                )
                .unwrap();
            self.ledger.put_result(&r).unwrap();
            Ok(v)
        }
    }

    const REPLAY: Policy = Policy {
        mode: Mode::Replay,
        proofs: 2,
        audit: 0.05,
        replay_failures: false,
    };

    /// The real case: a flaky test hangs to go test's timeout on the first
    /// execution. That failure is never replayed; the next execution
    /// disagrees, which quarantines the key; re-proof takes three identical
    /// runs (two plus one per divergence), then replay resumes.
    #[test]
    fn a_flaky_failure_seen_once_is_never_replayed() {
        let mut w = World::new();
        let hang = b"--- FAIL: TestHuhPrompterConfirm (600.00s)\npanic: test timed out after 10m0s\nFAIL\tgithub.com/cli/cli/v2/internal/prompter\t600.012s\n";
        let ok = b"ok  \tgithub.com/cli/cli/v2/internal/prompter\t0.412s\n";
        let v1 = w.run(&REPLAY, 1, hang).unwrap();
        assert_eq!(
            (v1.reason.clone(), v1.streak, v1.needed),
            (Reason::Miss, 0, 2),
            "a timed-out failure never counts as a proof"
        );
        assert!(!v1.replayable() && v1.compared_with.is_none());
        assert_eq!(
            v1.lines(),
            [
                "trust: not provable (its stdout says \"panic: test timed out after\": killed or timed out, not finished); it executes every time"
            ]
        );

        // Second run: executes (the hit is unprovable), passes: divergence.
        let v2 = w.run(&REPLAY, 0, ok).unwrap();
        assert!(
            matches!(v2.reason, Reason::Unprovable { .. }),
            "{:?}",
            v2.reason
        );
        let d = v2.divergence.clone().expect("a divergence");
        assert!(d.items[0].starts_with("exit code 1 -> 0"), "{d}");
        assert!(d.items[1].starts_with("stdout differs from line 1"), "{d}");
        assert_eq!(v2.trust.state, TrustState::Quarantined);
        assert_eq!(
            (v2.trust.audits, v2.trust.divergences, v2.streak, v2.needed),
            (1, 1, 1, 3)
        );
        assert!(
            v2.lines()[0].starts_with("QUOD  DIVERGENCE  step key quarantined"),
            "{:?}",
            v2.lines()
        );
        assert!(!v2.replayable());

        // Quarantined: executes and compares until three identical runs.
        let v3 = w
            .run(
                &REPLAY,
                0,
                b"ok  \tgithub.com/cli/cli/v2/internal/prompter\t0.399s\n",
            )
            .unwrap();
        assert!(
            matches!(
                v3.reason,
                Reason::Quarantined {
                    streak: 1,
                    needed: 3,
                    ..
                }
            ),
            "{:?}",
            v3.reason
        );
        assert!(
            v3.divergence.is_none() && !v3.byte_identical,
            "durations differ only"
        );
        assert_eq!((v3.streak, v3.trust.state), (2, TrustState::Quarantined));
        let v4 = w.run(&REPLAY, 0, ok).unwrap();
        assert_eq!((v4.streak, v4.trust.state), (3, TrustState::Proven));
        assert!(v4.replayable());
        assert!(v4.lines()[0].contains("proven again"), "{:?}", v4.lines());
        // Now it replays, and the receipt facts say what proved it.
        assert_eq!(w.run(&REPLAY, 0, ok), Err(Decision::Replay));
        let trust = Trust::new(&w.ledger, &w.cas, REPLAY).unwrap();
        let f = trust.facts(&w.key, &w.mid).unwrap();
        assert_eq!(
            f.receipt_line(),
            "trust: proven by 3 identical executions, audited 3 times, 1 divergence"
        );
        assert_eq!((f.trust.eligible, f.trust.replays), (1, 1));
        let audits = w.ledger.audits(Some(&w.key), 10).unwrap();
        assert_eq!(divergence_rate(&audits), (1, 3));
        assert_eq!(
            audits[2]
                .detail
                .as_deref()
                .unwrap()
                .split(';')
                .next()
                .unwrap(),
            "exit code 1 -> 0"
        );
    }

    /// A failure that repeats identically (a test hanging under load every
    /// time the box is busy) never proves itself by default; with
    /// --replay-failures a deterministic failure does, but output carrying
    /// a kill or timeout marker never does, even with exit 0.
    #[test]
    fn failures_and_kill_markers_never_prove() {
        let mut w = World::new();
        for i in 0..3 {
            let v = w.run(&REPLAY, 1, b"FAIL\tpkg\t0.1s\n").unwrap();
            assert!(!v.replayable() && v.divergence.is_none(), "run {i}: {v:?}");
            assert_eq!(v.streak, 0);
            assert!(
                v.lines()[0].starts_with("trust: not provable (exit code 1"),
                "{:?}",
                v.lines()
            );
        }
        let failures = Policy {
            replay_failures: true,
            ..REPLAY
        };
        let mut w = World::new();
        w.run(&failures, 1, b"FAIL\n").unwrap();
        assert!(w.run(&failures, 1, b"FAIL\n").unwrap().replayable());
        assert_eq!(w.run(&failures, 1, b"FAIL\n"), Err(Decision::Replay));
        // A default-policy run never serves that proven failure.
        assert!(matches!(
            w.run(&REPLAY, 1, b"FAIL\n").unwrap().reason,
            Reason::Unprovable { .. }
        ));

        let mut w = World::new();
        let swallowed = b"panic: test timed out after 10m0s\nok\n";
        for i in 0..3 {
            let v = w.run(&failures, 0, swallowed).unwrap();
            assert!(!v.replayable() && v.unprovable.is_some(), "run {i}: {v:?}");
        }
        for m in KILL_MARKERS {
            let mut w = World::new();
            let r = w.result(0, format!("x\n{m} y\n").as_bytes(), vec![]);
            assert!(unprovable(&failures, &r, &w.cas).unwrap().is_some(), "{m}");
        }
        let r = w.result(0, b"ok  \tpkg\t0.1s\n", vec![]);
        assert_eq!(unprovable(&REPLAY, &r, &w.cas).unwrap(), None);
    }

    /// A replay decided before another run quarantined the key (or took
    /// its audit slot) is decided again when it is recorded, and executes.
    #[test]
    fn a_replay_is_decided_again_when_it_is_recorded() {
        let mut w = World::new();
        let ok = b"ok\n";
        w.run(&REPLAY, 0, ok).unwrap();
        w.run(&REPLAY, 0, ok).unwrap();
        let hit = w
            .ledger
            .latest_result(&w.key, &w.mid)
            .unwrap()
            .map(|r| (w.manifest.clone(), r))
            .unwrap();
        let trust = Trust::new(&w.ledger, &w.cas, REPLAY).unwrap();
        assert_eq!(trust.decide(&w.key, Some(&hit)).unwrap(), Decision::Replay);
        // Meanwhile a parallel run quarantined the key.
        let mut t = w.ledger.trust(&w.key).unwrap().unwrap();
        t.state = TrustState::Quarantined;
        w.ledger.put_trust(&t).unwrap();
        let step = Step {
            key: w.key,
            cmd: "go test ./...",
            ignore: &[],
        };
        assert!(matches!(
            trust.record_replay(&step, &hit).unwrap(),
            Replayed::Execute(Reason::Quarantined { .. })
        ));
        assert_eq!(w.ledger.trust(&w.key).unwrap().unwrap().replays, 0);
        // Or took the audit slot: at rate 0.5 the second would-be hit is
        // audited; a parallel run already served the first.
        t.state = TrustState::Proven;
        w.ledger.put_trust(&t).unwrap();
        let half = Policy {
            audit: 0.5,
            ..REPLAY
        };
        let trust = Trust::new(&w.ledger, &w.cas, half).unwrap();
        assert_eq!(trust.decide(&w.key, Some(&hit)).unwrap(), Decision::Replay);
        assert!(matches!(
            trust.record_replay(&step, &hit).unwrap(),
            Replayed::Replayed(_)
        ));
        assert!(matches!(
            trust.record_replay(&step, &hit).unwrap(),
            Replayed::Execute(Reason::Audit { nth: 2 })
        ));
    }

    /// A step printing random bytes never reaches two identical executions.
    #[test]
    fn a_nondeterministic_step_never_becomes_replayable() {
        let mut w = World::new();
        for i in 0..6u8 {
            let out = [i, i.wrapping_mul(7), 0xff];
            let v = w.run(&REPLAY, 0, &out).unwrap();
            assert!(!v.replayable(), "run {i}: {v:?}");
            if i > 0 {
                assert!(v.divergence.is_some(), "run {i}");
                assert_eq!(v.trust.state, TrustState::Quarantined);
                assert_eq!(v.trust.divergences, u64::from(i));
                assert_eq!(
                    v.needed,
                    2 + u32::from(i),
                    "the bar rises with every divergence"
                );
            }
        }
        // Observe mode would never have replayed either, and says so.
        let v = w.run(&Policy::default(), 0, b"same").unwrap();
        assert!(!v.replayable() && v.divergence.is_some());
    }

    /// Observe mode executes everything, compares eligible hits (a free
    /// audit) and reports what it would have saved; a sampled audit in
    /// replay mode behaves the same.
    #[test]
    fn observe_mode_audits_every_eligible_hit() {
        let mut w = World::new();
        let observe = Policy::default();
        let ok = b"ok  \tpkg\t0.4s\n";
        let v = w.run(&observe, 0, ok).unwrap();
        assert_eq!(v.reason, Reason::Miss);
        let v = w.run(&observe, 0, ok).unwrap();
        assert!(matches!(
            v.reason,
            Reason::Proving {
                streak: 1,
                needed: 2
            }
        ));
        assert!(v.replayable() && v.byte_identical);
        assert_eq!(
            v.lines(),
            [
                "trust: proof 2 of 2, byte-identical to run 01RUN01; replayable from the next matching run"
            ]
        );
        let v = w.run(&observe, 0, ok).unwrap();
        assert_eq!(v.reason, Reason::WouldReplay);
        assert_eq!(v.would_have_saved, Some(Duration::from_secs(20)));
        assert_eq!(
            v.headline(),
            "  (observe mode: would have replayed, 20.0s not saved)"
        );
        assert_eq!(v.outcome(), "would-replay");
        assert_eq!(
            (v.trust.eligible, v.trust.audits, v.trust.replays),
            (1, 2, 0)
        );
        // A would-be hit that diverges in observe mode quarantines too.
        let v = w.run(&observe, 2, ok).unwrap();
        assert_eq!(v.outcome(), "diverged");
        assert_eq!(v.trust.state, TrustState::Quarantined);
        let audits = w.ledger.audits(Some(&w.key), 10).unwrap();
        assert_eq!(
            audits.iter().map(|a| a.phase.as_str()).collect::<Vec<_>>(),
            ["observe", "observe", "proof"]
        );

        // Replay mode, every hit audited: the audit executes and compares.
        let mut w = World::new();
        let all = Policy {
            audit: 1.0,
            ..REPLAY
        };
        w.run(&all, 0, ok).unwrap();
        w.run(&all, 0, ok).unwrap();
        let v = w.run(&all, 0, ok).unwrap();
        assert_eq!(v.reason, Reason::Audit { nth: 1 });
        assert_eq!(v.headline(), "  (audit: would-be hit 1 re-executed)");
        assert_eq!(
            v.lines(),
            ["trust: audit passed, byte-identical to run 01RUN02; audited 2 times, 0 divergences"]
        );
        // No audits: plain replay.
        let none = Policy {
            audit: 0.0,
            ..REPLAY
        };
        assert_eq!(w.run(&none, 0, ok), Err(Decision::Replay));
    }

    /// A step that does not finish is not a result and not a divergence (a
    /// timeout is as likely the box as the step), but the expected result
    /// did not reproduce, so the matched manifest proves again from 0.
    #[test]
    fn a_step_that_does_not_finish_restarts_its_streak() {
        let mut w = World::new();
        let ok = b"ok\n";
        w.run(&REPLAY, 0, ok).unwrap();
        w.run(&REPLAY, 0, ok).unwrap();
        let hit = w
            .ledger
            .latest_result(&w.key, &w.mid)
            .unwrap()
            .map(|r| (w.manifest.clone(), r));
        let trust = Trust::new(&w.ledger, &w.cas, REPLAY).unwrap();
        let step = Step {
            key: w.key,
            cmd: "go test ./...",
            ignore: &[],
        };
        let line = trust
            .record_incomplete(&step, hit.as_ref(), "timed out after 30m00s")
            .unwrap()
            .unwrap();
        assert!(
            line.starts_with("trust: the step timed out after 30m00s, which is not a result (run 01RUN02 finished with exit code 0)")
                && line.ends_with("proves again from 0 of 2"),
            "{line}"
        );
        let t = w.ledger.trust(&w.key).unwrap().unwrap();
        assert_eq!(
            (t.state, t.divergences, t.audits),
            (TrustState::Proven, 0, 1)
        );
        assert_eq!(w.ledger.proof(&w.key, &w.mid).unwrap().unwrap().streak, 0);
        assert!(
            w.ledger
                .audits(Some(&w.key), 10)
                .unwrap()
                .iter()
                .all(|a| !a.diverged)
        );
        // Nothing expected: nothing recorded.
        assert!(
            trust
                .record_incomplete(&step, None, "timed out")
                .unwrap()
                .is_none()
        );
        // The next run compares with the last finished one and re-proves.
        let v = w.run(&REPLAY, 0, ok).unwrap();
        assert!(
            matches!(
                v.reason,
                Reason::Proving {
                    streak: 0,
                    needed: 2
                }
            ),
            "{:?}",
            v.reason
        );
        assert_eq!((v.streak, v.divergence), (1, None));
        assert!(w.run(&REPLAY, 0, ok).unwrap().replayable());
    }

    /// A hit whose fresh execution produced nothing comparable but behaved
    /// otherwise (the hooks: it reached for the network offline) quarantines
    /// the key, and it proves again like after any divergence.
    #[test]
    fn a_divergence_without_a_result_quarantines() {
        let mut w = World::new();
        let ok = b"ok\n";
        w.run(&REPLAY, 0, ok).unwrap();
        w.run(&REPLAY, 0, ok).unwrap();
        let hit = w
            .ledger
            .latest_result(&w.key, &w.mid)
            .unwrap()
            .map(|r| (w.manifest.clone(), r))
            .unwrap();
        let trust = Trust::new(&w.ledger, &w.cas, REPLAY).unwrap();
        let step = Step {
            key: w.key,
            cmd: "go test ./...",
            ignore: &[],
        };
        let reason = Reason::Audit { nth: 1 };
        let v = trust
            .record_divergence(
                &step,
                "01FRESH",
                &hit,
                &reason,
                "tried to reach the network offline",
            )
            .unwrap();
        assert_eq!(v.outcome(), "diverged");
        assert_eq!(v.compared_with.as_deref(), Some("01RUN02"));
        let t = w.ledger.trust(&w.key).unwrap().unwrap();
        assert_eq!(t.state, TrustState::Quarantined);
        assert!(
            t.quarantine_reason
                .as_deref()
                .unwrap()
                .contains("tried to reach the network offline"),
            "{t:?}"
        );
        assert_eq!(w.ledger.proof(&w.key, &w.mid).unwrap().unwrap().streak, 0);
        let a = w.ledger.audits(Some(&w.key), 10).unwrap();
        assert!(
            a.iter()
                .any(|a| a.diverged && a.actual_run.as_deref() == Some("01FRESH"))
        );
        assert!(matches!(
            trust.decide(&w.key, Some(&hit)).unwrap(),
            Decision::Execute(Reason::Quarantined { .. })
        ));
    }

    /// Command files are part of the result: a different GITHUB_OUTPUT is
    /// a different result, and the same one fingerprints the same.
    #[test]
    fn command_files_are_compared() {
        let mut w = World::new();
        let mut a = w.result(0, b"ok\n", vec![]);
        let mut b = w.result(0, b"ok\n", vec![]);
        assert_eq!(
            fingerprint(&a, &w.cas, &[]).unwrap(),
            fingerprint(&b, &w.cas, &[]).unwrap()
        );
        let out = |v: &[u8]| quod_manifest::FileCommand {
            name: "GITHUB_OUTPUT".into(),
            h: w.cas.put(v).unwrap(),
        };
        a.commands = vec![out(b"x=1\n")];
        b.commands = vec![out(b"x=2\n")];
        let d = compare(&a, &b, &w.cas, &[]).unwrap();
        assert!(
            d.items
                .iter()
                .any(|i| i.contains("command files") && i.contains("GITHUB_OUTPUT")),
            "{d}"
        );
        assert_ne!(
            fingerprint(&a, &w.cas, &[]).unwrap(),
            fingerprint(&b, &w.cas, &[]).unwrap()
        );
        b.commands = a.commands.clone();
        assert!(compare(&a, &b, &w.cas, &[]).unwrap().is_empty());
    }

    /// The bar rises with recent divergences, at most MAX_EXTRA_PROOFS, and
    /// falls back as clean audits push them out of the window.
    #[test]
    fn the_bar_is_capped_and_decays() {
        let p = Policy::default();
        assert_eq!(needed(&p, 0), 2);
        assert_eq!(needed(&p, 3), 5);
        assert_eq!(needed(&p, 1_000), 2 + MAX_EXTRA_PROOFS);
        assert_eq!(needed(&p, u64::MAX), 2 + MAX_EXTRA_PROOFS);
        let big = Policy {
            proofs: MAX_PROOFS,
            ..p
        };
        assert!(big.validate().is_ok() && needed(&big, u64::MAX) == MAX_PROOFS + MAX_EXTRA_PROOFS);
        assert!(
            Policy {
                proofs: MAX_PROOFS + 1,
                ..p
            }
            .validate()
            .is_err()
        );

        // A chronically flaky key: many divergences, then it settles. It
        // re-proves within the capped bar, and once its divergences leave
        // the window the bar is the base again.
        let mut w = World::new();
        for i in 0..10u8 {
            w.run(&REPLAY, 0, &[i]).unwrap();
        }
        let v = w.run(&REPLAY, 0, b"settled").unwrap();
        assert_eq!(v.needed, 2 + MAX_EXTRA_PROOFS, "capped");
        let mut runs = 1;
        loop {
            match w.run(&REPLAY, 0, b"settled") {
                Err(Decision::Replay) => break,
                Ok(v) => {
                    assert!(v.divergence.is_none());
                    runs += 1;
                    assert!(runs <= 2 + MAX_EXTRA_PROOFS, "re-proves within the cap");
                }
                Err(d) => panic!("{d:?}"),
            }
        }
        assert_eq!(runs, 2 + MAX_EXTRA_PROOFS);
        let recent = w
            .ledger
            .recent_divergences(&w.key, TRAILING_AUDITS)
            .unwrap();
        assert_eq!(
            recent, 10,
            "nine between the random runs, one when it settled"
        );
        // Push the divergences out of the window with clean audits.
        let all = Policy {
            audit: 1.0,
            ..REPLAY
        };
        for _ in 0..TRAILING_AUDITS {
            assert!(w.run(&all, 0, b"settled").unwrap().divergence.is_none());
        }
        assert_eq!(
            w.ledger
                .recent_divergences(&w.key, TRAILING_AUDITS)
                .unwrap(),
            0
        );
        let trust = Trust::new(&w.ledger, &w.cas, REPLAY).unwrap();
        assert_eq!(trust.facts(&w.key, &w.mid).unwrap().needed, 2);
    }

    /// Outputs under the toolchain's cache directories are not compared;
    /// anything else is, down to a mode bit.
    #[test]
    fn compare_names_every_difference_outside_ignored_outputs() {
        let mut w = World::new();
        let ignore = vec![".quod/gocache".to_string()];
        let a = w.result(
            0,
            b"ok  \tpkg\t0.4s\n",
            vec![
                Entry::file(".quod/gocache/trim.txt", Hash::of(b"1"), 0o644),
                Entry::file("out/bin", Hash::of(b"bin"), 0o755),
                Entry::dir("out", Hash::of(b"")),
            ],
        );
        let b = w.result(
            0,
            b"ok  \tpkg\t9.9s\n",
            vec![
                Entry::file(".quod/gocache/trim.txt", Hash::of(b"2"), 0o644),
                Entry::file("out/bin", Hash::of(b"bin"), 0o755),
                Entry::dir("out", Hash::of(b"")),
            ],
        );
        assert!(compare(&a, &b, &w.cas, &ignore).unwrap().is_empty());
        assert_eq!(
            fingerprint(&a, &w.cas, &ignore).unwrap(),
            fingerprint(&b, &w.cas, &ignore).unwrap()
        );
        assert_ne!(
            fingerprint(&a, &w.cas, &[]).unwrap(),
            fingerprint(&b, &w.cas, &[]).unwrap(),
            "without the ignore list trim.txt counts"
        );
        let c = w.result(
            3,
            b"ok  \tpkg\t0.4s\nextra\n",
            vec![
                Entry::file("out/bin", Hash::of(b"bin"), 0o644),
                Entry::file("out/new", Hash::of(b"n"), 0o644),
            ],
        );
        let d = compare(&a, &c, &w.cas, &ignore).unwrap();
        assert_eq!(
            d.items,
            [
                "exit code 0 -> 3",
                "stdout differs from line 2: \"<end of output>\" vs \"extra\"",
                "outputs differ at 3 path(s): out missing, out/bin changed, out/new new",
            ]
        );
        // Writes outside the workspace (rootfs mode) are compared exactly.
        let mut e = w.result(0, b"ok  \tpkg\t0.4s\n", a.outputs.clone());
        e.root_outputs = vec![Entry::file("opt/marker", Hash::of(b"1"), 0o644)];
        let mut f = e.clone();
        f.root_outputs = vec![Entry::file("opt/marker", Hash::of(b"2"), 0o644)];
        assert_eq!(
            compare(&e, &f, &w.cas, &ignore).unwrap().items,
            ["writes outside the workspace differ at 1 path(s): opt/marker changed"]
        );
        assert_ne!(
            fingerprint(&e, &w.cas, &ignore).unwrap(),
            fingerprint(&f, &w.cas, &ignore).unwrap()
        );
    }

    /// Two read sets of one tree are equivalent only when they differ in
    /// absent probes a recorded listing already pins.
    #[test]
    fn equivalent_read_sets_differ_only_in_pinned_absent_probes() {
        let m = |entries: Vec<Entry>| Manifest {
            v: 1,
            step_key: Hash::of(b"k"),
            roots: vec!["/workspace".into()],
            entries,
            volatile: vec![],
        };
        let base = vec![
            Entry::dir("pkg", Hash::of(b"listing")),
            Entry::file("pkg/a.go", Hash::of(b"a"), 0o644),
            Entry::absent("pkg/textflag.h"),
        ];
        let a = m(base.clone());
        assert_eq!(equivalent(&a, &a), Ok(()));
        // The probe skipped (a cached asm action): pinned by `pkg`'s listing.
        let b = m(base[..2].to_vec());
        assert_eq!(equivalent(&a, &b), Ok(()));
        assert_eq!(equivalent(&b, &a), Ok(()));
        // A file read by one execution only is a different read set.
        let c = m(vec![base[0].clone(), base[2].clone()]);
        assert_eq!(
            equivalent(&a, &c),
            Err("pkg/a.go read by the earlier execution only".into())
        );
        // An absent probe in a directory nobody listed is not pinned.
        let mut e = base.clone();
        e.push(Entry::absent("other/x.h"));
        assert_eq!(
            equivalent(&a, &m(e)),
            Err("other/x.h read by this execution only".into())
        );
        // Same path, other content.
        let mut f = base.clone();
        f[1] = Entry::file("pkg/a.go", Hash::of(b"a2"), 0o644);
        assert_eq!(
            equivalent(&a, &m(f)),
            Err("pkg/a.go recorded differently".into())
        );
    }

    /// The hit's manifest matched the tree, but the execution read another
    /// set of inputs: a divergence, unless the sets are equivalent.
    #[test]
    fn a_different_read_set_on_a_matching_tree_diverges_unless_equivalent() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path().join("cas")).unwrap();
        let ledger = Ledger::open_in_memory().unwrap();
        let key = Hash::of(b"go test ./...");
        let tree = Tree::from_leaves([
            Entry::file("pkg/a.go", Hash::of(b"a"), 0o644),
            Entry::file("pkg/b.go", Hash::of(b"b"), 0o644),
        ]);
        let acc = |p: &str, kind: Kind| Access {
            path: p.into(),
            kind,
        };
        let man = |log: Vec<Access>| {
            Manifest::from_log(key, vec!["/workspace".into()], &tree, log).unwrap()
        };
        let hit_m = man(vec![
            acc("pkg", Kind::Dir),
            acc("pkg/a.go", Kind::File),
            acc("pkg/textflag.h", Kind::Absent),
        ]);
        let eq_m = man(vec![acc("pkg", Kind::Dir), acc("pkg/a.go", Kind::File)]);
        let neq_m = man(vec![
            acc("pkg", Kind::Dir),
            acc("pkg/a.go", Kind::File),
            acc("pkg/b.go", Kind::File),
        ]);
        let trust = Trust::new(&ledger, &cas, REPLAY).unwrap();
        let step = Step {
            key,
            cmd: "go test ./...",
            ignore: &[],
        };
        let mut n = 0;
        let mut exec = |m: &Manifest, hit: Option<&(Manifest, StepResult)>, reason: Reason| {
            n += 1;
            let mid = ledger.put_manifest(m, "r").unwrap();
            let r = World::mk(&cas, key, mid, n, 0, b"ok\n", vec![]);
            let v = trust
                .record_execution(&step, &r.run_id, m, &r, hit, &reason)
                .unwrap();
            ledger.put_result(&r).unwrap();
            (v, r)
        };
        let (_, r1) = exec(&hit_m, None, Reason::Miss);
        let hit = (hit_m.clone(), r1);
        let proving = Reason::Proving {
            streak: 1,
            needed: 2,
        };
        let (v, _) = exec(&eq_m, Some(&hit), proving.clone());
        assert_eq!(
            v.divergence, None,
            "equivalent: absent probe pinned by the listing"
        );
        assert_eq!(
            (v.streak, v.trust.audits),
            (1, 1),
            "a new manifest starts its own streak"
        );
        let (v, _) = exec(&neq_m, Some(&hit), proving);
        let d = v.divergence.expect("a divergence");
        assert!(
            d.items[0].starts_with("inputs differed")
                && d.items[0].ends_with("(pkg/b.go read by this execution only)"),
            "{d}"
        );
        assert_eq!(v.trust.state, TrustState::Quarantined);
    }

    /// A result recorded before the trust layer (no proof row) is compared
    /// with the next execution but never counted as a proof.
    #[test]
    fn a_result_from_before_the_trust_layer_is_compared_not_trusted() {
        let mut w = World::new();
        let r0 = World::mk(&w.cas, w.key, w.mid, 99, 0, b"ok\n", vec![]);
        w.ledger.put_manifest(&w.manifest, "v1").unwrap();
        w.ledger.put_result(&r0).unwrap();
        let v = w.run(&REPLAY, 0, b"ok\n").unwrap();
        assert!(
            matches!(
                v.reason,
                Reason::Proving {
                    streak: 0,
                    needed: 2
                }
            ),
            "{:?}",
            v.reason
        );
        assert_eq!(
            (v.streak, v.divergence.is_none(), v.trust.audits),
            (1, true, 1)
        );
        // Its next execution makes two; then it replays.
        assert!(w.run(&REPLAY, 0, b"ok\n").unwrap().replayable());
        assert_eq!(w.run(&REPLAY, 0, b"ok\n"), Err(Decision::Replay));

        // A v1 result that a fresh execution contradicts is a divergence.
        let mut w = World::new();
        let r0 = World::mk(&w.cas, w.key, w.mid, 99, 1, b"FAIL\n", vec![]);
        w.ledger.put_manifest(&w.manifest, "v1").unwrap();
        w.ledger.put_result(&r0).unwrap();
        let v = w.run(&REPLAY, 0, b"ok\n").unwrap();
        assert!(v.divergence.is_some(), "{v:?}");
        assert_eq!(v.trust.state, TrustState::Quarantined);
    }

    /// The comparison rules (normalize, the ignored outputs, HOME_VOLATILE)
    /// can change between executions. Stored fingerprints were computed
    /// under the old rules; the previous result is fingerprinted again
    /// under the new ones, so a rules change is never a divergence. When the
    /// previous result is gone (purged), a mismatch with its stored
    /// fingerprint restarts the streak instead.
    #[test]
    fn a_change_in_the_comparison_rules_never_quarantines() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path().join("cas")).unwrap();
        let ledger = Ledger::open_in_memory().unwrap();
        let key = Hash::of(b"go test ./...");
        let tree = Tree::from_leaves([Entry::file("go.mod", Hash::of(b"m"), 0o644)]);
        let m = Manifest::from_log(
            key,
            vec!["/workspace".into()],
            &tree,
            [Access {
                path: "go.mod".into(),
                kind: Kind::File,
            }],
        )
        .unwrap();
        let mid = manifest_id(&m).unwrap();
        let outputs = vec![
            Entry::dir("out", Hash::of(b"listing")),
            Entry::file("out/x", Hash::of(b"x"), 0o644),
        ];
        let trust = Trust::new(&ledger, &cas, REPLAY).unwrap();
        let mut n = 0;
        let mut exec = |ignore: &[String]| {
            n += 1;
            let hit = ledger
                .latest_result(&key, &mid)
                .unwrap()
                .map(|r| (m.clone(), r));
            let reason = match trust.decide(&key, hit.as_ref()).unwrap() {
                Decision::Execute(r) => r,
                Decision::Replay => Reason::WouldReplay,
            };
            let r = World::mk(&cas, key, mid, n, 0, b"ok\n", outputs.clone());
            let step = Step {
                key,
                cmd: "go test ./...",
                ignore,
            };
            ledger.put_manifest(&m, &r.run_id).unwrap();
            let v = trust
                .record_execution(&step, &r.run_id, &m, &r, hit.as_ref(), &reason)
                .unwrap();
            ledger
                .record_run(&r.run_id, None, None, quod_ledger::Observed::default())
                .unwrap();
            ledger.put_result(&r).unwrap();
            v
        };
        let old_rules: Vec<String> = vec![];
        let new_rules = vec!["out".to_string()];
        exec(&old_rules);
        assert_eq!(exec(&old_rules).streak, 2);
        // The rules change (out/ is now left out of comparisons): the
        // stored fingerprint covered out/x, the new one does not.
        let v = exec(&new_rules);
        assert_eq!((v.divergence, v.streak), (None, 3), "the streak goes on");
        assert_eq!(v.trust.state, TrustState::Proven);
        // Back again, the other way: still no divergence.
        let v = exec(&old_rules);
        assert_eq!((v.divergence, v.streak), (None, 4));
        // The previous results are purged (their transport was): only the
        // stored fingerprint is left, from other rules. No divergence; the
        // manifest proves again.
        assert!(ledger.purge_transport(None).unwrap() >= 4);
        let v = exec(&new_rules);
        assert_eq!(v.divergence, None);
        assert_eq!(v.streak, 1, "a mismatch with a stored fingerprint restarts");
        assert_ne!(v.trust.state, TrustState::Quarantined);
        assert_eq!(ledger.trust(&key).unwrap().unwrap().divergences, 0);
    }

    /// A proof whose run never got its result stored (a crash between the
    /// two) vouches for nothing: the older result is not replayed on it.
    #[test]
    fn a_proof_without_its_result_does_not_replay_an_older_one() {
        let mut w = World::new();
        let ok = b"ok\n";
        w.run(&REPLAY, 0, ok).unwrap();
        w.run(&REPLAY, 0, ok).unwrap();
        // A third execution whose proof was written but whose result was not.
        let trust = Trust::new(&w.ledger, &w.cas, REPLAY).unwrap();
        let hit = w
            .ledger
            .latest_result(&w.key, &w.mid)
            .unwrap()
            .map(|r| (w.manifest.clone(), r));
        let lost = World::mk(&w.cas, w.key, w.mid, 50, 0, ok, vec![]);
        trust
            .record_execution(
                &Step {
                    key: w.key,
                    cmd: "go test ./...",
                    ignore: &[],
                },
                &lost.run_id,
                &w.manifest,
                &lost,
                hit.as_ref(),
                &Reason::WouldReplay,
            )
            .unwrap();
        assert_eq!(w.ledger.proof(&w.key, &w.mid).unwrap().unwrap().streak, 3);
        assert!(matches!(
            trust.decide(&w.key, hit.as_ref()).unwrap(),
            Decision::Execute(Reason::Proving { streak: 0, .. })
        ));
    }
}
