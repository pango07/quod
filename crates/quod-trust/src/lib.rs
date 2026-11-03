//! The trust layer: when a recorded result may be replayed, how a step key
//! earns that, and how it loses it.
//!
//! A matching manifest is necessary for a replay but not sufficient. The
//! result must also be *provable* and *proven*: the same manifest must have
//! produced the same result `proofs` executions in a row (the proof streak,
//! kept per (key, manifest) in the ledger's `proofs` table). Trust v2
//! (docs/trust-v2.md) sets `proofs` to 1: a result recorded once may replay
//! on the next identical tree, and the evidence that the *step* is
//! deterministic is pooled across every manifest of its key by the audit
//! schedule: the key's first `shadow` would-be hits are all audited (the
//! shadow phase), the rate then decays as 1/k to the `audit` floor. Nothing
//! of a key replays blind until the key has agreed with itself `shadow`
//! times. Never replayed, by default, is any failure (non-zero exit; a
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
//! reproduced `proofs + divergences` times in a row, the shadow phase
//! restarts and lengthens, the floor rises, every replay the key served
//! since its last quarantine is marked `suspect` in the ledger (it may have
//! been wrong; re-run it), and the divergence is reported loudly. In observe
//! mode every eligible hit is compared this way (an audit for free, since
//! the step executes anyway); in replay mode the schedule says which
//! eligible hits are re-executed instead of replayed. Audit never turns off
//! unless the rate is set to 0 on purpose.
//!
//! The ledger stores the facts; this crate decides. Nothing here knows about
//! FUSE, sandboxes or the command line.

use std::collections::BTreeMap;
use std::fmt;
use std::io::Read;
use std::str::FromStr;
use std::time::Duration;

use quod_cas::{BlobStore, CanonError, Cas, CasError, Entry, Hash, Kind, Tree, parent_and_name};
use quod_ledger::{
    AuditRow, Backlog, Ledger, LedgerError, LedgerStore, Lookup, NewPending, ProofRow, RunRecord,
    TrustRow, TrustState, now_rfc3339,
};
use quod_manifest::{Manifest, ManifestError, StepResult, manifest_id};
use serde::{Deserialize, Serialize};

/// Identical executions a manifest needs before its result may replay.
/// One (trust v2): a result recorded once replays on the next identical
/// tree; the shadow audits below are what stands between a step's first
/// record and its first blind replay.
pub const DEFAULT_PROOFS: u32 = 1;
/// Would-be hits of a key that are all audited before any of it replays
/// blind (trust v2's shadow phase), counted from the key's last quarantine.
pub const DEFAULT_SHADOW: u32 = 2;
/// The largest `--shadow` accepted.
pub const MAX_SHADOW: u32 = 1000;
/// The fraction of would-be hits replay mode re-executes and compares once
/// the shadow phase and its 1/k decay are over: the audit floor.
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
/// Deferred audits (docs/trust-v2.md, "Deferred audits"): how many audits
/// of one key may be owed (served, not yet executed) before the next due
/// audit runs inline again. Bounds the exposure window in hits: at most
/// this many audit intervals of a key are served without a verdict.
pub const MAX_DEFERRED_PER_KEY: u64 = 2;
/// A deferred audit nobody executed within this long is owed as if lost
/// (the worker was never started, or the box was busy for a day): the
/// decision counts it so, whether or not a worker has marked it.
pub const PENDING_TTL: Duration = Duration::from_secs(24 * 3600);
/// A claimed deferred audit whose worker has not finished it within this
/// long was abandoned (the worker died, or the step hung): released for
/// another attempt. Also the most one attempt may run.
pub const LEASE: Duration = Duration::from_secs(3 * 3600);
/// Attempts before a deferred audit that keeps failing to execute is lost.
pub const MAX_ATTEMPTS: u32 = 3;

/// Now minus `PENDING_TTL`, as the ledger's timestamps are written: a
/// pending audit enqueued before it is overdue.
pub fn stale_cutoff() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    quod_ledger::format_rfc3339(now.saturating_sub(PENDING_TTL).as_secs(), 0)
}

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

/// Workspace paths left out when two results are compared (still recorded,
/// still restored by a replay), on the same terms as `HOME_VOLATILE`: each
/// one was seen differing between two executions on identical inputs, and
/// what differs is not a result. Hypothesis's Unicode tables
/// (python-attrs/attrs on bench-real-small, fix/replay-gaps): it writes them
/// with `gzip.GzipFile(tmpfile, "wb", mtime=1)` to a fresh `mkstemp` name and
/// renames, so the gzip header's file name field is random on every run,
/// while the content is a pure function of the interpreter's `unicodedata`
/// (keyed: the interpreter is in the host fingerprint or the rootfs) and
/// Hypothesis reads only the decompressed content. Hypothesis's example
/// database (`.hypothesis/examples`), which changes what later runs try,
/// stays compared.
pub const WORKSPACE_VOLATILE: &[&str] = &[".hypothesis/unicode_data"];

/// `observe` executes everything and reports what would have replayed;
/// `replay` replays proven results.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Policy {
    pub mode: Mode,
    /// Identical executions a manifest needs (plus one per past divergence
    /// of its key) before it may replay. At least 1.
    pub proofs: u32,
    /// Would-be hits of a key audited in full before any replays blind,
    /// counted from the key's last quarantine (times `1 + recent
    /// divergences`). 0: no shadow phase, the floor from the first hit.
    #[serde(default = "default_shadow")]
    pub shadow: u32,
    /// The audit floor: the fraction of would-be hits re-executed in replay
    /// mode once the shadow phase has decayed, 0..=1. 0 turns every audit
    /// off, the shadow included (benchmarks only).
    pub audit: f64,
    /// Whether a failing result (non-zero exit) may prove itself and
    /// replay. Off by default: a failure that repeats identically is as
    /// likely the box (load, a hang to go test's timeout) as the code.
    pub replay_failures: bool,
    /// Deferred audits (trust v2, "Deferred audits"): a decay-phase audit
    /// serves the replay at once and is executed and compared later, off
    /// the job's critical path; a divergence then marks the replays served
    /// meanwhile suspect. Shadow audits are inline whatever this says. Off:
    /// every audit executes inline, as before. Absent on the wire (a
    /// client before deferred audits): off. quod-server never defers.
    #[serde(default)]
    pub defer: bool,
}

fn default_shadow() -> u32 {
    DEFAULT_SHADOW
}

impl Default for Policy {
    fn default() -> Policy {
        Policy {
            mode: Mode::Observe,
            proofs: DEFAULT_PROOFS,
            shadow: DEFAULT_SHADOW,
            audit: DEFAULT_AUDIT,
            replay_failures: false,
            defer: true,
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
        if self.shadow > MAX_SHADOW {
            return Err(TrustError::BadPolicy(format!(
                "--shadow {} is more than {MAX_SHADOW}",
                self.shadow
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
    /// A remote step ledger (quod-server) could not be used.
    #[error("quod server: {0}")]
    Remote(String),
    /// What a deferred audit stores could not be made (the input tree's
    /// document, the request); the replay it belonged to is rolled back.
    #[error("deferred audit: {0}")]
    Defer(String),
}

/// Why a step executes although the ledger was consulted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
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
    /// Replay mode: this would-be hit (the `nth` for the key) is audited;
    /// `shadow` when it falls in the key's shadow phase (every hit audited
    /// until the key has agreed with itself `shadow` times).
    Audit { nth: u64, shadow: bool },
    /// A deferred audit being executed now: would-be hit `nth` of the key
    /// was served as a replay, and this is its execution (pending audit
    /// `pending` in the ledger). Compared like an audit; not a would-be hit
    /// of its own (the replay already counted).
    DeferredAudit { nth: u64, pending: i64 },
    /// Observe mode: this would have replayed.
    WouldReplay,
}

impl Reason {
    /// Is this execution evidence about a recorded result: compared with it
    /// (an audit, shadow or decay, or an observe-mode would-be hit, which
    /// counts toward the shadow like an audit) or a proof toward it (a
    /// manifest proving itself, a key re-proving after a quarantine)? Such
    /// an execution must run the step now: a caller whose tool can serve
    /// results from a cache of its own (go test's, under Quod's guard)
    /// expires it here. A miss and an unprovable result are not: they only
    /// record, and are checked by the executions above.
    pub fn is_evidence(&self) -> bool {
        matches!(
            self,
            Reason::Audit { .. }
                | Reason::DeferredAudit { .. }
                | Reason::WouldReplay
                | Reason::Proving { .. }
                | Reason::Quarantined { .. }
        )
    }

    /// Does this execution count as a would-be hit?
    fn eligible(&self) -> bool {
        matches!(self, Reason::Audit { .. } | Reason::WouldReplay)
    }

    fn phase(&self) -> &'static str {
        match self {
            Reason::Audit { shadow: true, .. } => "shadow",
            Reason::Audit { .. } => "audit",
            Reason::DeferredAudit { .. } => "deferred",
            Reason::WouldReplay => "observe",
            _ => "proof",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Decision {
    Replay,
    /// Replay now, and audit later: the would-be hit (the `nth` of the key)
    /// is on the audit schedule, in the decay phase, and the policy defers.
    /// The caller serves the replay and enqueues the audit in the same
    /// transaction (`Ledger::enqueue_audit`).
    Defer {
        nth: u64,
    },
    Execute(Reason),
}

/// What `Trust::record_replay` concluded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Replayed {
    /// Recorded; the receipt's trust facts.
    Replayed(Facts),
    /// Recorded, and the caller owes a deferred audit of would-be hit `nth`:
    /// it must enqueue one in the same transaction.
    Deferred(Facts, u64),
    /// Decided otherwise inside the transaction: execute, for this reason.
    Execute(Reason),
}

/// Identical executions a key's manifests need: the policy's count plus one
/// for every divergence among the key's newest `TRAILING_AUDITS` audits, at
/// most `MAX_EXTRA_PROOFS` more. A key that keeps diverging needs longer
/// runs of identical results; one that stops diverging gets back to the
/// base bar as clean audits push its divergences out of the window.
pub fn needed(policy: &Policy, recent_divergences: u64) -> u32 {
    policy.proofs.saturating_add(extra(recent_divergences))
}

/// How many of a key's recent divergences count: at most `MAX_EXTRA_PROOFS`.
fn extra(recent_divergences: u64) -> u32 {
    u32::try_from(recent_divergences)
        .unwrap_or(u32::MAX)
        .min(MAX_EXTRA_PROOFS)
}

/// The audit schedule of a key (docs/trust-v2.md, "The schedule"): the
/// first `shadow` would-be hits since the key's last quarantine are all
/// audited, then the weight of hit k decays as `shadow / k` down to `floor`.
/// After d recent divergences (capped like the bar) the shadow is
/// `shadow * (1 + d)` long and the floor `floor * (1 + d)`, at most 1.
/// A policy with `audit` 0 has no audits at all.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Schedule {
    pub shadow: u32,
    pub floor: f64,
}

impl Schedule {
    pub fn of(policy: &Policy, recent_divergences: u64) -> Schedule {
        let rate = policy.audit.clamp(0.0, 1.0);
        if rate <= 0.0 {
            return Schedule {
                shadow: 0,
                floor: 0.0,
            };
        }
        let m = 1 + extra(recent_divergences);
        Schedule {
            shadow: policy.shadow.saturating_mul(m),
            floor: (rate * f64::from(m)).min(1.0),
        }
    }

    /// The audit weight of the k-th would-be hit since the shadow began
    /// (k >= 1): the probability-like share of such hits that are audited.
    pub fn weight(&self, k: u64) -> f64 {
        if self.floor <= 0.0 || k == 0 {
            return 0.0;
        }
        if k <= u64::from(self.shadow) {
            1.0
        } else {
            (f64::from(self.shadow) / k as f64).clamp(self.floor, 1.0)
        }
    }

    /// The weights of hits 1..=n summed. Beyond `shadow / floor` every
    /// weight is the floor, so the sum is closed-form from there.
    fn cumulative(&self, n: u64) -> f64 {
        if self.floor <= 0.0 {
            return 0.0;
        }
        let tail_from = (f64::from(self.shadow) / self.floor).ceil() as u64;
        let head = n.min(tail_from);
        let mut s = 0.0;
        for k in 1..=head {
            s += self.weight(k);
        }
        s + (n - head) as f64 * self.floor
    }

    /// Whether the next would-be hit (the `k_before + 1`th since the shadow
    /// began) is audited: deterministic, hit k is audited exactly when the
    /// cumulative weight crosses an integer, so a given policy yields the
    /// same schedule on every ledger and tests can rely on it. With no
    /// shadow this is "every 1/floor-th hit", v1's schedule. A step cannot
    /// game it: it never sees the count.
    pub fn due(&self, k_before: u64) -> bool {
        if self.floor <= 0.0 {
            return false;
        }
        let f = |s: f64| (s + 1e-9).floor();
        f(self.cumulative(k_before + 1)) > f(self.cumulative(k_before))
    }

    /// Whether the next would-be hit falls in the shadow phase.
    pub fn in_shadow(&self, k_before: u64) -> bool {
        k_before < u64::from(self.shadow)
    }

    /// Would-be hits left in the shadow phase after `k_done` of them.
    pub fn shadow_left(&self, k_done: u64) -> u64 {
        u64::from(self.shadow).saturating_sub(k_done)
    }
}

/// How many would-be hits of a key count toward its shadow phase: those
/// since its last quarantine.
fn since_shadow(trust: Option<&TrustRow>) -> u64 {
    trust.map_or(0, |t| t.eligible.saturating_sub(t.shadow_from))
}

/// What the comparisons so far say about a step's output (docs/trust-v2.md,
/// "Determinism classification").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Determinism {
    /// No comparison yet.
    Unknown,
    /// Every comparison was byte-identical.
    Exact,
    /// Some comparison was identical only after `normalize`: the output
    /// carries recognized volatile fields (durations, timestamps). Reported,
    /// not penalized: the volatility is understood and erased.
    Normalized,
    /// A divergence among the key's newest `TRAILING_AUDITS` audits: the
    /// stricter schedule and bar apply until it ages out.
    Divergent,
}

impl Determinism {
    pub fn as_str(self) -> &'static str {
        match self {
            Determinism::Unknown => "unknown",
            Determinism::Exact => "exact",
            Determinism::Normalized => "normalized",
            Determinism::Divergent => "divergent",
        }
    }

    /// One clause for `status` and `inspect`.
    pub fn describe(self) -> &'static str {
        match self {
            Determinism::Unknown => "not compared yet",
            Determinism::Exact => "every comparison byte-identical",
            Determinism::Normalized => {
                "output carries volatile fields (durations, timestamps), erased before comparing"
            }
            Determinism::Divergent => {
                "diverged recently: longer shadow, higher audit floor and bar until it ages out"
            }
        }
    }
}

pub fn determinism(trust: &TrustRow, recent_divergences: u64) -> Determinism {
    if recent_divergences > 0 {
        Determinism::Divergent
    } else if trust.audits == 0 {
        Determinism::Unknown
    } else if trust.volatile > 0 {
        Determinism::Normalized
    } else {
        Determinism::Exact
    }
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
/// never replays, whatever its streak. `backlog` is the key's deferred
/// audits: one that was lost is repaid by auditing the next would-be hit
/// inline, schedule or not; and a due decay-phase audit is deferred only
/// while fewer than `MAX_DEFERRED_PER_KEY` are already owed.
pub fn decide(
    policy: &Policy,
    hit: Option<HitView>,
    trust: Option<&TrustRow>,
    proof: Option<&ProofRow>,
    recent_divergences: u64,
    backlog: Backlog,
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
            let k = since_shadow(trust);
            let sched = Schedule::of(policy, recent_divergences);
            let nth = eligible + 1;
            if backlog.owed > 0 && sched.floor > 0.0 {
                // A deferred audit of this key was lost (or is overdue):
                // this hit makes up for it, inline, whatever the schedule
                // says; in the shadow it is a shadow audit like any other.
                return Decision::Execute(Reason::Audit {
                    nth,
                    shadow: sched.in_shadow(k),
                });
            }
            if !sched.due(k) {
                return Decision::Replay;
            }
            let shadow = sched.in_shadow(k);
            if !shadow && policy.defer && backlog.pending < MAX_DEFERRED_PER_KEY {
                Decision::Defer { nth }
            } else {
                Decision::Execute(Reason::Audit { nth, shadow })
            }
        }
    }
}

/// v1's schedule, kept for callers that reason about a flat rate: whether
/// the next would-be hit (the `eligible_before + 1`th) is audited at `rate`
/// with no shadow phase, i.e. every `1/rate`th one.
pub fn audit_due(eligible_before: u64, rate: f64) -> bool {
    Schedule {
        shadow: 0,
        floor: rate.clamp(0.0, 1.0),
    }
    .due(eligible_before)
}

// ----- comparing results -----

/// Erases what two faithful executions print differently in stdout and
/// stderr (docs/trust-v2.md, "Normalizations"): go test's per-package
/// durations (`ok  \tpkg\t0.412s`, `FAIL\tpkg\t1.2s`, or `(cached)` where go
/// test served the result from its own cache, with any fields after it,
/// such as coverage, kept) and per-test durations (`--- PASS: TestX
/// (0.00s)`); the number of the go command's temporary build dir in a `go
/// env` `GOGCCFLAGS=` line (setup-go prints `go env` on every run); ISO 8601
/// timestamps with a time of day (`2026-11-02T09:12:03Z`,
/// `2026-11-02 09:12:03.007+01:00`; a bare date is kept, it is as likely a
/// version as the clock); and a test runner's duration, a number with a
/// time unit inside parentheses (`(12ms)`, `(1.23 s)`) or after `in`,
/// `took`, `Time`, `Duration`, `Finished` (`12 passed in 3.21s`, `Time:
/// 1.234 s`). Only the number and unit go, so `1 passed in 3s` and `2 passed
/// in 3s` still differ. Nothing else: exit codes, package and test names,
/// failure output and counts all stay, and file outputs are never
/// normalized. A line of any other shape is kept byte for byte.
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
    canonical_parallel_order(out)
}

/// Parallel test runners report results in completion order, which
/// changes from run to run on identical inputs (the Moto benchmark's
/// `pytest -n 4` diverged on every audit). In output that is unmistakably
/// such a runner's, each run of consecutive per-result lines is put in a
/// canonical order that keeps everything but the order: the multiset of
/// results (and so every count), each line's position-bound fields, and
/// every other line where it was. Never weaker than comparing the multiset
/// of per-test results plus the summary: a test that passes in one run and
/// fails in the other changes a line's status (and the summary).
///
/// - pytest-xdist (a `created: N/N workers`, `N workers [M items]` or, under
///   `-q`, `bringing up nodes...` line):
///   `-v` result lines `[gw2] [ 25%] PASSED tests/a.py::t` become
///   `[gw*] [ 25%] ` + the sorted `STATUS nodeid` of the run (the worker
///   is whichever was free; the percentages are the completion count, the
///   same sequence every run); worker lines `[gw0] linux -- Python 3.12`
///   are sorted within their run; the progress lines `....s.F.. [ 13%]`
///   (no file name: xdist cannot say which file a result is from) keep
///   their lengths and suffixes and get the run's status characters in
///   sorted order (which test a character stands for is not printed, so
///   only the counts were ever there to compare).
/// - jest (a `Test Suites: ` summary line): runs of consecutive suite lines
///   `PASS src/a.test.js` / `FAIL src/b.test.js` are sorted (workers finish
///   suites in any order).
///
/// Output without the marker is untouched, so a sequential runner's order
/// (which can be a result) is compared as printed.
fn canonical_parallel_order(out: Vec<u8>) -> Vec<u8> {
    let Ok(text) = std::str::from_utf8(&out) else {
        return out;
    };
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    fn body(l: &str) -> &str {
        l.strip_suffix('\n').unwrap_or(l).trim_end_matches('\r')
    }
    let xdist = lines.iter().any(|l| is_xdist_header(body(l)));
    let jest = lines.iter().any(|l| body(l).starts_with("Test Suites: "));
    if !xdist && !jest {
        return out;
    }
    #[derive(PartialEq, Clone, Copy)]
    enum Kind {
        Other,
        Result,
        Worker,
        Progress,
        Suite,
        Summary,
    }
    // pytest's short test summary (`-r`): one line per result or group of
    // skips, in the order the reports arrived, which xdist scrambles.
    let mut in_summary = vec![false; lines.len()];
    let mut open = false;
    for (n, l) in lines.iter().enumerate() {
        let b = body(l);
        if b.starts_with('=') {
            open = b.contains(" short test summary info ");
        } else {
            in_summary[n] = open;
        }
    }
    let kind = |n: usize| {
        let b = body(lines[n]);
        if xdist && in_summary[n] && is_summary_line(b) {
            return Kind::Summary;
        }
        if xdist {
            if xdist_result(b).is_some() || xdist_start(b) {
                return Kind::Result;
            }
            if xdist_worker(b) {
                return Kind::Worker;
            }
            if xdist_progress(b).is_some() {
                return Kind::Progress;
            }
        }
        if jest && (b.starts_with("PASS ") || b.starts_with("FAIL ")) {
            return Kind::Suite;
        }
        Kind::Other
    };
    let mut res = String::with_capacity(text.len());
    let mut i = 0;
    while i < lines.len() {
        let k = kind(i);
        let mut j = i + 1;
        while k != Kind::Other && j < lines.len() && kind(j) == k {
            j += 1;
        }
        let run = &lines[i..j];
        // Each line keeps its own line ending.
        let ends: Vec<&str> = run.iter().map(|l| &l[body(l).len()..]).collect();
        match k {
            Kind::Other => res.push_str(lines[i]),
            Kind::Result => {
                // `-v` without a terminal interleaves each test's start
                // line (its node id) with the results: the starts first,
                // sorted, then the results, sorted, with the percentage
                // sequence as printed.
                let mut starts: Vec<(&str, &str)> = Vec::new();
                let mut results: Vec<(&str, &str)> = Vec::new();
                for (l, e) in run.iter().zip(&ends) {
                    match xdist_result(body(l)) {
                        Some((_, r)) => results.push((r, e)),
                        None => starts.push((body(l), e)),
                    }
                }
                let pcts: Vec<&str> = run
                    .iter()
                    .filter_map(|l| xdist_result(body(l)).map(|r| r.0))
                    .collect();
                starts.sort_unstable();
                results.sort_unstable();
                for (s, e) in starts {
                    res.push_str(s);
                    res.push_str(e);
                }
                for ((r, e), pct) in results.into_iter().zip(pcts) {
                    res.push_str(&format!("[gw*] {pct} {r}{e}"));
                }
            }
            Kind::Worker | Kind::Suite | Kind::Summary => {
                let mut b: Vec<&str> = run.iter().map(|l| body(l)).collect();
                b.sort_unstable();
                for (x, e) in b.iter().zip(&ends) {
                    res.push_str(x);
                    res.push_str(e);
                }
            }
            Kind::Progress => {
                let mut chars: Vec<char> = run
                    .iter()
                    .flat_map(|l| xdist_progress(body(l)).unwrap().0.chars())
                    .collect();
                chars.sort_unstable();
                let mut it = chars.into_iter();
                for (l, e) in run.iter().zip(&ends) {
                    let (marks, suffix) = xdist_progress(body(l)).unwrap();
                    res.extend(it.by_ref().take(marks.chars().count()));
                    res.push_str(suffix);
                    res.push_str(e);
                }
            }
        }
        i = j;
    }
    res.into_bytes()
}

/// A line of pytest's short test summary: `SKIPPED [2] tests/a.py:12:
/// reason`, `FAILED tests/a.py::t - AssertionError`, `XFAIL ...`.
fn is_summary_line(b: &str) -> bool {
    [
        "PASSED ", "FAILED ", "ERROR ", "SKIPPED ", "XFAIL ", "XPASS ",
    ]
    .iter()
    .any(|p| b.starts_with(p))
}

/// `created: 4/4 workers`, `4 workers [530 items]`, `1 worker [1 item]`.
fn is_xdist_header(b: &str) -> bool {
    // What xdist prints instead under `-q`.
    if b == "bringing up nodes..." {
        return true;
    }
    if let Some(rest) = b.strip_prefix("created: ") {
        return rest
            .strip_suffix(" workers")
            .or_else(|| rest.strip_suffix(" worker"))
            .and_then(|n| n.split_once('/'))
            .is_some_and(|(a, c)| is_digits(a) && is_digits(c));
    }
    let Some((n, rest)) = b.split_once(' ') else {
        return false;
    };
    is_digits(n)
        && (rest.starts_with("workers [") || rest.starts_with("worker ["))
        && (rest.ends_with(" items]") || rest.ends_with(" item]"))
}

fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// `[gwN] ` at the start of `b`: the rest.
fn after_worker(b: &str) -> Option<&str> {
    let rest = b.strip_prefix("[gw")?;
    let close = rest.find("] ")?;
    is_digits(&rest[..close]).then(|| &rest[close + 2..])
}

/// An xdist `-v` result line, `[gw2] [ 25%] PASSED tests/a.py::t `: its
/// percentage field (`[ 25%]`) and the rest (`PASSED tests/a.py::t `).
fn xdist_result(b: &str) -> Option<(&str, &str)> {
    let rest = after_worker(b)?;
    let close = rest.find("%] ")?;
    let pct = &rest[..close + 2];
    let n = pct.strip_prefix('[')?.trim_start();
    let n = n.strip_suffix("%]")?;
    let status = &rest[close + 3..];
    let ok = is_digits(n)
        && [
            "PASSED ", "FAILED ", "SKIPPED ", "ERROR ", "XFAIL ", "XPASS ",
        ]
        .iter()
        .any(|s| status.starts_with(s));
    ok.then_some((pct, status))
}

/// An xdist `-v` start line: a node id and a space, `tests/a.py::T::t[x] `.
fn xdist_start(b: &str) -> bool {
    b.ends_with(' ')
        && !b.starts_with(['[', ' ', '='])
        && b.split_once("::")
            .is_some_and(|(file, _)| !file.contains(' ') && !file.is_empty())
}

/// A worker line that is not a result: `[gw0] linux -- Python 3.12.1 ...`,
/// `[gw1] Python 3.12.1 (main) -- [GCC 13]`.
fn xdist_worker(b: &str) -> bool {
    after_worker(b).is_some_and(|r| !r.starts_with('['))
}

/// An xdist progress line, `....s.F..          [ 13%]`: its status
/// characters and the rest (spaces and the percentage).
fn xdist_progress(b: &str) -> Option<(&str, &str)> {
    let n = b.bytes().take_while(|c| b".sFExX".contains(c)).count();
    if n == 0 {
        return None;
    }
    let suffix = &b[n..];
    let pct = suffix.trim_start();
    let inner = pct.strip_prefix('[')?.strip_suffix("%]")?.trim_start();
    (suffix.starts_with(' ') && is_digits(inner)).then_some((&b[..n], suffix))
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
    if let Some(masked) = mask_go_package_line(s, "\t") {
        return masked.into_bytes();
    }
    // go test -json: an event's `"Elapsed":0.412`, and the package and
    // per-test lines its `"Output"` carries (JSON-escaped: `\t`, `\n`).
    let json = mask_go_json_event(s);
    let s = json.as_deref().unwrap_or(s);
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
    // Trust v2: timestamps and test-runner durations, anywhere in a line;
    // then the shapes particular to one runner (node:test, vitest, pytest's
    // clock after its duration).
    let masked = mask_vitest(&mask_node_test(&mask_clock_after_duration(
        &mask_runner_durations(&mask_timestamps(s)),
    )));
    if masked != s || json.is_some() {
        return masked.into_bytes();
    }
    line.to_vec()
}

/// A go test package line with its fields separated by `sep` (a tab, or
/// the two bytes `\t` inside go test -json's escaped `Output`), its
/// duration erased: `ok  \tpkg\t0.412s`, `FAIL\tpkg\t1.2s`, or `(cached)`
/// where go test served the result from its cache (a warm step under
/// Quod's guard, quod_key::gotest). The duration may be followed by a note
/// in the same field (`0.001s [no tests to run]`, `(cached) [no tests to
/// run]`) or by more fields (`\tcoverage: 81.2% of statements`,
/// `\tcoverage: 0.0% of statements [no tests to run]`), all kept. `None`
/// for any other line, among them `?   \tpkg\t[no test files]` (it has no
/// duration) and `FAIL\tpkg [build failed]`.
fn mask_go_package_line(s: &str, sep: &str) -> Option<String> {
    if !(s.starts_with(&format!("ok  {sep}")) || s.starts_with(&format!("FAIL{sep}"))) {
        return None;
    }
    let mut fields: Vec<&str> = s.split(sep).collect();
    let third = *fields.get(2)?;
    let (head, tail) = match third.find(' ') {
        Some(i) => third.split_at(i),
        None => (third, ""),
    };
    if !(is_duration(head) || head == "(cached)") {
        return None;
    }
    let masked = format!("<dur>{tail}");
    fields[2] = &masked;
    Some(fields.join(sep))
}

/// A go test -json event line (`{"Time":"...","Action":"pass",...}`) with
/// its `"Elapsed":<seconds>` erased and the package line its `"Output"`
/// may carry masked as `mask_go_package_line` masks one (a per-test
/// `--- PASS: T (0.00s)` inside it goes with the parenthesized durations,
/// the `Time` with the timestamps). `None` for any other line.
fn mask_go_json_event(s: &str) -> Option<String> {
    if !(s.starts_with("{\"Time\":\"") || s.starts_with("{\"Action\":\""))
        || !s.contains("\"Action\":\"")
        || !s.ends_with('}')
    {
        return None;
    }
    let mut out = s.to_string();
    const ELAPSED: &str = "\"Elapsed\":";
    if let Some(i) = out.find(ELAPSED) {
        let from = i + ELAPSED.len();
        let n = number_len(&out.as_bytes()[from..]);
        if n > 0 {
            out.replace_range(from..from + n, "\"<dur>\"");
        }
    }
    const OUTPUT: &str = "\"Output\":\"";
    if let Some(i) = out.find(OUTPUT) {
        let from = i + OUTPUT.len();
        let body = &out[from..];
        // The line itself, up to its escaped newline or the string's end.
        if let Some(end) = body.find("\\n").or_else(|| body.find('"'))
            && let Some(masked) = mask_go_package_line(&body[..end], "\\t")
        {
            out.replace_range(from..from + end, &masked);
        }
    }
    Some(out)
}

/// node:test's durations, which carry no unit: the spec reporter's summary
/// `ℹ duration_ms 1541.8`, the TAP reporter's `# duration_ms 1541.8` and
/// its per-test YAML `duration_ms: 0.4567`. The number after the word
/// `duration_ms` (an optional `:`, then spaces) becomes `<dur>`; the
/// per-test spec lines (`✔ name (1.2ms)`) go with the parenthesized
/// durations.
fn mask_node_test(s: &str) -> String {
    const WORD: &str = "duration_ms";
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find(WORD) {
        let b = rest.as_bytes();
        let boundary = i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_');
        let mut j = i + WORD.len();
        if b.get(j) == Some(&b':') {
            j += 1;
        }
        let spaces = b[j..].iter().take_while(|c| **c == b' ').count();
        let num = number_len(&b[j + spaces..]);
        if boundary && spaces > 0 && num > 0 {
            out.push_str(&rest[..j + spaces]);
            out.push_str("<dur>");
            rest = &rest[j + spaces + num..];
        } else {
            out.push_str(&rest[..i + WORD.len()]);
            rest = &rest[i + WORD.len()..];
        }
    }
    out.push_str(rest);
    out
}

/// The length of a plain number at the start of `b` (digits, an optional
/// fraction) that ends there: followed by the end or a byte that is not
/// alphanumeric, `.` or `_`. 0 if there is none.
fn number_len(b: &[u8]) -> usize {
    let int = b.iter().take_while(|c| c.is_ascii_digit()).count();
    if int == 0 {
        return 0;
    }
    let mut i = int;
    if b.get(i) == Some(&b'.') {
        let frac = b[i + 1..].iter().take_while(|c| c.is_ascii_digit()).count();
        if frac == 0 {
            return 0;
        }
        i += 1 + frac;
    }
    match b.get(i) {
        Some(c) if c.is_ascii_alphanumeric() || *c == b'.' || *c == b'_' => 0,
        _ => i,
    }
}

/// `H:MM:SS` (or `HH:MM:SS`) at the start of `b`, followed by the end or a
/// byte that is not alphanumeric: its length.
fn clock_len(b: &[u8]) -> Option<usize> {
    let d = |i: usize| b.get(i).is_some_and(u8::is_ascii_digit);
    let h = b.iter().take_while(|c| c.is_ascii_digit()).count();
    if !(h == 1 || h == 2) {
        return None;
    }
    let ok = b.get(h) == Some(&b':')
        && d(h + 1)
        && d(h + 2)
        && b.get(h + 3) == Some(&b':')
        && d(h + 4)
        && d(h + 5)
        && !b.get(h + 6).is_some_and(u8::is_ascii_alphanumeric);
    ok.then_some(h + 6)
}

/// pytest's long-run summary prints the clock after the duration: `in
/// 62.01s (0:01:02)`. Once the duration is `<dur>`, a parenthesized clock
/// right after it goes too.
fn mask_clock_after_duration(s: &str) -> String {
    const AFTER: &str = "<dur> (";
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find(AFTER) {
        let from = i + AFTER.len();
        match clock_len(&rest.as_bytes()[from..]) {
            Some(n) if rest.as_bytes().get(from + n) == Some(&b')') => {
                out.push_str(&rest[..from]);
                out.push_str("<dur>");
                rest = &rest[from + n..];
            }
            _ => {
                out.push_str(&rest[..from]);
                rest = &rest[from..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// vitest's timing lines, beyond what the generic rules erase: the summary
/// `Duration  1.23s (transform 45ms, setup 0ms, collect 100ms, tests 20ms,
/// environment 0ms, prepare 50ms)` (every duration on a line that starts
/// with `Duration`), `Start at  09:12:03` (a clock time after `Start at`),
/// and a test file's line `✓ src/a.test.ts (3 tests | 1 skipped) 5ms` (a
/// duration ending the line right after a parenthesized test count).
fn mask_vitest(s: &str) -> String {
    let t = s.trim_start();
    let lead = &s[..s.len() - t.len()];
    if t.starts_with("Duration ") {
        return format!("{lead}{}", mask_every_duration(t));
    }
    if let Some(rest) = t.strip_prefix("Start at ") {
        let spaces = rest.bytes().take_while(|c| *c == b' ').count();
        if let Some(n) = clock_len(&rest.as_bytes()[spaces..]) {
            return format!(
                "{lead}Start at {}<ts>{}",
                &rest[..spaces],
                &rest[spaces + n..]
            );
        }
        return s.to_string();
    }
    if let Some(close) = s.rfind(") ")
        && let Some(open) = s[..close].rfind('(')
    {
        let tail = &s[close + 2..];
        let count = &s[open + 1..close];
        if duration_len(tail.as_bytes()) == Some(tail.len())
            && count.bytes().next().is_some_and(|c| c.is_ascii_digit())
            && count
                .trim_start_matches(|c: char| c.is_ascii_digit())
                .starts_with(" test")
        {
            return format!("{}<dur>", &s[..close + 2]);
        }
    }
    s.to_string()
}

/// `s` with every duration (`duration_len`) that starts a word replaced by
/// `<dur>`.
fn mask_every_duration(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let (mut i, mut from) = (0, 0);
    while i < b.len() {
        let word_start = i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'.');
        if word_start
            && b[i].is_ascii_digit()
            && let Some(n) = duration_len(&b[i..])
        {
            out.push_str(&s[from..i]);
            out.push_str("<dur>");
            i += n;
            from = i;
        } else {
            i += 1;
        }
    }
    out.push_str(&s[from..]);
    out
}

/// `0.412s`, `600.00s`: digits with an optional point, then `s`.
fn is_duration(t: &str) -> bool {
    t.strip_suffix('s').is_some_and(|n| {
        !n.is_empty()
            && n.bytes().all(|b| b.is_ascii_digit() || b == b'.')
            && n.parse::<f64>().is_ok()
    })
}

/// The length of an ISO 8601 timestamp with a time of day at the start of
/// `b`: `YYYY-MM-DD` then `T` or one space, `HH:MM:SS`, an optional
/// `.fraction`, an optional `Z` or `±HH:MM`; followed by a non-alphanumeric
/// byte or the end. `None` if `b` does not start with one.
fn timestamp_len(b: &[u8]) -> Option<usize> {
    let digits = |from: usize, n: usize| {
        b.len() >= from + n && b[from..from + n].iter().all(u8::is_ascii_digit)
    };
    let at = |i: usize, c: u8| b.get(i) == Some(&c);
    if !(digits(0, 4) && at(4, b'-') && digits(5, 2) && at(7, b'-') && digits(8, 2)) {
        return None;
    }
    if !(at(10, b'T') || at(10, b' ')) {
        return None;
    }
    if !(digits(11, 2) && at(13, b':') && digits(14, 2) && at(16, b':') && digits(17, 2)) {
        return None;
    }
    let mut i = 19;
    if at(i, b'.') {
        let frac = b[i + 1..].iter().take_while(|c| c.is_ascii_digit()).count();
        if frac == 0 {
            return None;
        }
        i += 1 + frac;
    }
    if at(i, b'Z') {
        i += 1;
    } else if (at(i, b'+') || at(i, b'-'))
        && digits(i + 1, 2)
        && at(i + 3, b':')
        && digits(i + 4, 2)
    {
        i += 6;
    }
    match b.get(i) {
        Some(c) if c.is_ascii_alphanumeric() => None,
        _ => Some(i),
    }
}

/// `s` with every ISO 8601 timestamp (with a time of day) replaced by
/// `<ts>`. A timestamp must not follow a digit (so `12026-11-02T...` is
/// not one).
fn mask_timestamps(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let mut from = 0;
    while i < b.len() {
        let boundary = i == 0 || !b[i - 1].is_ascii_digit();
        if boundary
            && b[i].is_ascii_digit()
            && let Some(n) = timestamp_len(&b[i..])
        {
            out.push_str(&s[from..i]);
            out.push_str("<ts>");
            i += n;
            from = i;
        } else {
            i += 1;
        }
    }
    out.push_str(&s[from..]);
    out
}

/// Time units a test runner prints a duration in, longest first so that
/// `seconds` is tried before `s`.
const DURATION_UNITS: &[&str] = &[
    "milliseconds",
    "millisecond",
    "seconds",
    "second",
    "minutes",
    "minute",
    "msec",
    "mins",
    "secs",
    "sec",
    "min",
    "ms",
    "s",
    "m",
];

/// The length of a duration at the start of `b`: digits with an optional
/// fraction, at most one space, a unit from `DURATION_UNITS`, then a
/// non-alphanumeric byte or the end.
fn duration_len(b: &[u8]) -> Option<usize> {
    let int = b.iter().take_while(|c| c.is_ascii_digit()).count();
    if int == 0 {
        return None;
    }
    let mut i = int;
    if b.get(i) == Some(&b'.') {
        let frac = b[i + 1..].iter().take_while(|c| c.is_ascii_digit()).count();
        if frac == 0 {
            return None;
        }
        i += 1 + frac;
    }
    if b.get(i) == Some(&b' ') {
        i += 1;
    }
    let rest = &b[i..];
    let unit = DURATION_UNITS.iter().find(|u| {
        rest.starts_with(u.as_bytes())
            && !rest.get(u.len()).is_some_and(|c| c.is_ascii_alphanumeric())
    })?;
    Some(i + unit.len())
}

/// Words after which a number with a time unit is a duration a test runner
/// printed (`12 passed in 3.21s`, `took 450ms`, `Time: 1.234 s`,
/// `Duration  1.23s`, `Finished in 0.5 seconds`). An optional `:` and
/// spaces may follow the word.
const DURATION_WORDS: &[&str] = &["in", "took", "Time", "Duration", "Finished", "estimated"];

/// `s` with every test-runner duration replaced by `<dur>`: a duration in
/// parentheses, or one after a `DURATION_WORDS` word at a word boundary.
fn mask_runner_durations(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let mut from = 0;
    while i < b.len() {
        // `(12ms)`, `(1.23 s)`.
        if b[i] == b'('
            && let Some(n) = duration_len(&b[i + 1..])
            && b.get(i + 1 + n) == Some(&b')')
        {
            out.push_str(&s[from..i]);
            out.push_str("(<dur>)");
            i += n + 2;
            from = i;
            continue;
        }
        let word_start = i == 0 || !b[i - 1].is_ascii_alphanumeric();
        if word_start
            && let Some(w) = DURATION_WORDS
                .iter()
                .find(|w| b[i..].starts_with(w.as_bytes()))
        {
            let mut j = i + w.len();
            // `in` must end the word (`in` not `inputs`); then `:`? spaces.
            if b.get(j).is_some_and(|c| c.is_ascii_alphanumeric()) {
                i += 1;
                continue;
            }
            if b.get(j) == Some(&b':') {
                j += 1;
            }
            let spaces = b[j..].iter().take_while(|c| **c == b' ').count();
            if spaces > 0
                && let Some(n) = duration_len(&b[j + spaces..])
            {
                out.push_str(&s[from..j + spaces]);
                out.push_str("<dur>");
                i = j + spaces + n;
                from = i;
                continue;
            }
            i = j;
            continue;
        }
        i += 1;
    }
    out.push_str(&s[from..]);
    out
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

fn read_blob(cas: &(impl BlobStore + ?Sized), h: &Hash) -> Result<Vec<u8>, TrustError> {
    let mut buf = Vec::new();
    cas.open_blob_reader(h)?.read_to_end(&mut buf)?;
    Ok(buf)
}

/// What a result is compared on, hashed: exit code, normalized stdout and
/// stderr, comparable outputs, and every write outside the workspace
/// (`root_outputs`, rootfs mode: compared exactly, none left out). Two
/// results with equal fingerprints are identical for the trust layer.
pub fn fingerprint(
    r: &StepResult,
    cas: &(impl BlobStore + ?Sized),
    ignore: &[String],
) -> Result<Hash, TrustError> {
    let out = Hash::of(&normalize(&read_blob(cas, &r.stdout)?));
    let err = Hash::of(&normalize(&read_blob(cas, &r.stderr)?));
    let outputs =
        Hash::of(quod_cas::canonical_json(&comparable_outputs(&r.outputs, ignore))?.as_bytes());
    let root = Hash::of(quod_cas::canonical_json(&r.root_outputs)?.as_bytes());
    let mut buf = Vec::with_capacity(256);
    // v2: command files hashed in, GOGCCFLAGS' build dir normalized. v3:
    // the rename to Quod (the tag only; the rules are v2's). v4: a package
    // line's `(cached)` reads as a duration, and its duration is erased
    // when more fields follow it (exp/never-worse). v5: trust v2,
    // timestamps and test-runner durations normalized. v6: every go test
    // package-line form ([no tests to run], go test -json), node:test's
    // duration_ms, vitest's and jest's remaining timing fields
    // (fix/replay-gaps).
    buf.extend_from_slice(b"quod result fingerprint v6\0");
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
    cas: &(impl BlobStore + ?Sized),
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
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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
    cas: &(impl BlobStore + ?Sized),
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Earlier replays of the key this execution's divergence marked
    /// suspect (trust v2): they may have served a wrong result.
    pub suspects: usize,
    /// Would-be hits still in the key's shadow phase after this execution:
    /// how many more are audited before any replays blind.
    pub shadow_left: u64,
    /// This inline audit made up for a deferred audit of the key that was
    /// lost (never executed). Never on a server's verdict (it defers none).
    #[serde(default)]
    pub repaid: bool,
    /// Deferred audits of the key still outstanding at this execution's
    /// divergence, dropped: they would have compared against the result
    /// the quarantine has just declared suspect.
    #[serde(default)]
    pub superseded: usize,
}

impl Verdict {
    /// `; its next 2 would-be hits are audited`, or nothing once the shadow
    /// phase is over.
    fn shadow_note(&self) -> String {
        match self.shadow_left {
            0 => String::new(),
            1 => "; its next would-be hit is audited".to_string(),
            n => format!("; its next {n} would-be hits are audited"),
        }
    }

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
            Reason::Audit { .. } | Reason::DeferredAudit { .. } => "audit",
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
            (Reason::Audit { nth, shadow: true }, _) => {
                format!("  (shadow audit: would-be hit {nth} re-executed)")
            }
            (Reason::Audit { nth, .. }, _) => format!("  (audit: would-be hit {nth} re-executed)"),
            (Reason::DeferredAudit { nth, pending }, _) => format!(
                "  (deferred audit {pending}: would-be hit {nth} re-executed after its replay was served)"
            ),
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
            if self.suspects > 0 {
                v.push(format!(
                    "  {} earlier replay{} of this key may have served a wrong result: marked suspect in the ledger; re-run {} (`quod status` lists them)",
                    self.suspects,
                    plural(self.suspects as u64),
                    if self.suspects == 1 { "it" } else { "them" }
                ));
            }
            if self.superseded > 0 {
                v.push(format!(
                    "  {} deferred audit{} of this key still outstanding dropped: {} would have compared against the result now suspect; the restarted shadow checks the key instead",
                    self.superseded,
                    plural(self.superseded as u64),
                    if self.superseded == 1 { "it" } else { "they" }
                ));
            }
            return v;
        }
        let identical = match &self.compared_with {
            Some(r) => format!(
                ", {} run {}",
                if self.byte_identical {
                    "byte-identical to"
                } else {
                    "identical (up to normalized volatile fields) to"
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
            Reason::Miss if self.compared_with.is_none() && !self.replayable() => vec![format!(
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
                    format!(
                        "; replayable from the next matching run{}",
                        self.shadow_note()
                    )
                } else {
                    String::new()
                }
            )],
            Reason::Quarantined { since, why, .. } => vec![format!(
                "trust: quarantined since {} ({}); re-proof {} of {}{identical}{}",
                human_time(since),
                why,
                self.streak,
                self.needed,
                if self.replayable() {
                    format!(
                        "; proven again, replayable from the next matching run{}",
                        self.shadow_note()
                    )
                } else {
                    String::new()
                }
            )],
            Reason::Audit { shadow: true, .. } => vec![format!(
                "trust: shadow audit passed{identical}{}; {audited}",
                match self.shadow_left {
                    0 =>
                        "; the shadow phase is over, replays follow the audit schedule".to_string(),
                    1 => "; one more shadow audit before replays".to_string(),
                    n => format!("; {n} more shadow audits before replays"),
                }
            )],
            Reason::Audit { .. } => vec![format!(
                "trust: audit passed{identical}{}; {audited}",
                if self.repaid {
                    " (making up for a lost deferred audit)"
                } else {
                    ""
                }
            )],
            Reason::DeferredAudit { .. } => vec![format!(
                "trust: deferred audit passed{identical}; the replay it checked was right; {audited}"
            )],
            Reason::WouldReplay => vec![format!(
                "trust: {audited}; `quod run --mode replay` replays this"
            )],
        }
    }
}

// ----- the transitions: pure -----

/// What an execution's comparisons found, gathered from the ledger and the
/// blob store by `Trust::record_execution` (the only part that does I/O).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Compared {
    /// Every difference found so far (inputs not `equivalent` to the hit's,
    /// result differences against the hit and against the previous
    /// execution of this manifest).
    pub divergence: Divergence,
    /// The run this execution was compared with, if any.
    pub compared_with: Option<String>,
    /// Whether every compared result was byte-identical before normalizing.
    pub byte_identical: bool,
    /// The previous execution of this manifest, if the manifest has a proof
    /// row: its fingerprint recomputed under the current rules, or `None`
    /// when its result is gone (then only the stored fingerprint is left).
    pub previous: Option<Option<Hash>>,
}

/// The inputs of `execution_transition`: the stored facts about the key and
/// what this execution's comparisons found. Nothing in it is read lazily.
#[derive(Clone, Debug)]
pub struct ExecutionInput<'a> {
    pub key: Hash,
    pub cmd: &'a str,
    pub run_id: &'a str,
    /// The manifest this execution read, and its result's fingerprint.
    pub manifest: Hash,
    pub fingerprint: Hash,
    /// Why its result can never prove itself, if it cannot.
    pub unprovable: Option<String>,
    pub reason: &'a Reason,
    /// The key's trust row and this manifest's proof row, as stored.
    pub trust: Option<TrustRow>,
    pub prior: Option<ProofRow>,
    pub compared: Compared,
    /// Divergences among the key's newest `TRAILING_AUDITS - 1` audits
    /// (this execution's own audit row is the window's last).
    pub recent_divergences: u64,
    /// The recorded duration of the result a replay would have served.
    pub hit_duration_ms: Option<u64>,
    /// The time a quarantine starts, if this execution starts one.
    pub now: String,
}

/// What recording an execution changes, for the shell to write in this
/// order: every proof of the key reset (`reset_proofs`), this manifest's
/// proof row, the trust row, the audit row, and then, if `mark_suspects`,
/// the key's replays since its last quarantine marked suspect (trust v2;
/// the shell fills `verdict.suspects` with how many it marked).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionTransition {
    pub reset_proofs: bool,
    pub proof: ProofRow,
    pub trust: TrustRow,
    pub audit: Option<AuditRow>,
    pub mark_suspects: bool,
    pub verdict: Verdict,
}

/// The trust transition of one execution. Pure: the same input gives the
/// same transition on a laptop's ledger and on quod-server's, which is the
/// point: the local path and the server share this function, so their
/// trust rules cannot drift apart.
///
/// The streak of this manifest advances when its previous execution had
/// the same result, and starts at 1 when it was not seen before. Any
/// difference is a divergence: the key is quarantined, every streak of it
/// restarts (this execution's being the first of its new streak), its
/// shadow phase starts over and its earlier replays become suspect. A
/// comparison identical only after `normalize` counts toward the key's
/// `normalized` determinism class. An unprovable result is compared like
/// any other but never counts. Every comparison is an audit row.
pub fn execution_transition(policy: &Policy, input: ExecutionInput) -> ExecutionTransition {
    let ExecutionInput {
        key,
        cmd,
        run_id,
        manifest,
        fingerprint: fp,
        unprovable: why_unprovable,
        reason,
        trust,
        prior,
        compared,
        recent_divergences,
        hit_duration_ms,
        now,
    } = input;
    let Compared {
        mut divergence,
        mut compared_with,
        mut byte_identical,
        previous,
    } = compared;
    let mut trust = trust.unwrap_or_else(|| TrustRow::new(key, Some(cmd)));
    if trust.cmd.is_none() {
        trust.cmd = Some(cmd.to_string());
    }
    if reason.eligible() {
        trust.eligible += 1;
    }
    // Only a stored fingerprint was compared (the previous result is
    // purged): nothing was normalized, so not a `volatile` sighting.
    let mut fingerprint_only = false;
    let mut streak = 1;
    if let Some(p) = &prior {
        let same = match previous.flatten() {
            Some(pf) => {
                if pf != fp && divergence.is_empty() {
                    // compare() found nothing to name, yet the fingerprints
                    // differ under the same rules: say so rather than call
                    // them identical.
                    divergence
                        .push("result fingerprint differs from the previous execution".into());
                }
                pf == fp
            }
            None => {
                // Equal stored fingerprints still say the results agree (on
                // the old rules and the new alike); that counts as a
                // comparison. A mismatch says nothing.
                let same = p.fingerprint == fp;
                byte_identical = false;
                if same {
                    fingerprint_only = compared_with.is_none();
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
    let diverged = !divergence.is_empty();
    if compared {
        trust.audits += 1;
        if !diverged && !byte_identical && !fingerprint_only {
            // Identical only up to `normalize`: the output carries
            // recognized volatile fields (the `normalized` class).
            trust.volatile += 1;
        }
    }
    if diverged {
        trust.divergences += 1;
        trust.state = TrustState::Quarantined;
        trust.quarantined_at = Some(now);
        trust.quarantine_reason = Some(truncate(&divergence.to_string(), 300));
        // The shadow phase starts over: the key's next would-be hits are
        // all audited again, more of them (Schedule::of).
        trust.shadow_from = trust.eligible;
        // Every manifest of the key starts proving again; this one's new
        // result is the first of its new streak.
        streak = 1;
    }
    if why_unprovable.is_some() {
        // Compared like any result (a later different one is still a
        // divergence), but it never counts toward a replay.
        streak = 0;
    }
    let proof = ProofRow {
        step_key: key,
        manifest_id: manifest,
        streak,
        fingerprint: fp,
        last_run: run_id.to_string(),
        updated_at: String::new(),
    };
    // This execution's audit row is the window's newest: count it now.
    let recent = recent_divergences + u64::from(diverged);
    let needed = needed(policy, recent);
    if !diverged
        && why_unprovable.is_none()
        && streak >= needed
        && trust.state != TrustState::Proven
    {
        trust.state = TrustState::Proven;
        trust.quarantined_at = None;
        trust.quarantine_reason = None;
    }
    let audit = compared.then(|| AuditRow {
        id: 0,
        step_key: key,
        manifest_id: manifest,
        at: String::new(),
        phase: reason.phase().into(),
        diverged,
        expected_run: compared_with.clone(),
        actual_run: Some(run_id.to_string()),
        detail: Some(if !diverged {
            if byte_identical {
                "byte-identical".to_string()
            } else if fingerprint_only {
                "identical by stored fingerprint (previous result purged)".to_string()
            } else {
                "identical up to normalized volatile fields".to_string()
            }
        } else {
            truncate(&divergence.to_string(), 2000)
        }),
    });
    let shadow_left =
        Schedule::of(policy, recent).shadow_left(trust.eligible.saturating_sub(trust.shadow_from));
    let verdict = Verdict {
        reason: reason.clone(),
        compared_with,
        divergence: diverged.then_some(divergence),
        byte_identical,
        streak,
        needed,
        trust: trust.clone(),
        would_have_saved: match reason {
            Reason::WouldReplay => hit_duration_ms.map(Duration::from_millis),
            _ => None,
        },
        unprovable: why_unprovable,
        suspects: 0,
        shadow_left,
        repaid: false,
        superseded: 0,
    };
    ExecutionTransition {
        reset_proofs: diverged,
        proof,
        trust,
        mark_suspects: diverged && compared,
        audit,
        verdict,
    }
}

/// The inputs of `divergence_transition`.
#[derive(Clone, Debug)]
pub struct DivergenceInput<'a> {
    pub key: Hash,
    pub cmd: &'a str,
    pub run_id: &'a str,
    /// The hit's manifest and the run that recorded its result.
    pub manifest: Hash,
    pub hit_run: &'a str,
    pub reason: &'a Reason,
    /// How this execution behaved otherwise than the hit's run.
    pub why: &'a str,
    pub trust: Option<TrustRow>,
    /// Divergences among the key's newest `TRAILING_AUDITS - 1` audits.
    pub recent_divergences: u64,
    pub now: String,
}

/// What a divergence without a result changes: every proof of the key is
/// reset, then the trust row and the audit row are written, then the key's
/// replays since its last quarantine are marked suspect (the shell fills
/// `verdict.suspects`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DivergenceTransition {
    pub trust: TrustRow,
    pub audit: AuditRow,
    pub verdict: Verdict,
}

/// The trust transition of an execution of a hit that produced no result
/// to compare yet behaved otherwise than the run a replay would serve. Pure.
pub fn divergence_transition(policy: &Policy, input: DivergenceInput) -> DivergenceTransition {
    let mut trust = input
        .trust
        .unwrap_or_else(|| TrustRow::new(input.key, Some(input.cmd)));
    if trust.cmd.is_none() {
        trust.cmd = Some(input.cmd.to_string());
    }
    if input.reason.eligible() {
        trust.eligible += 1;
    }
    let mut divergence = Divergence::default();
    divergence.push(format!("this execution {}", input.why));
    trust.audits += 1;
    trust.divergences += 1;
    trust.state = TrustState::Quarantined;
    trust.quarantined_at = Some(input.now);
    trust.quarantine_reason = Some(truncate(&divergence.to_string(), 300));
    trust.shadow_from = trust.eligible;
    let audit = AuditRow {
        id: 0,
        step_key: input.key,
        manifest_id: input.manifest,
        at: String::new(),
        phase: input.reason.phase().into(),
        diverged: true,
        expected_run: Some(input.hit_run.to_string()),
        actual_run: Some(input.run_id.to_string()),
        detail: Some(truncate(&divergence.to_string(), 2000)),
    };
    // This execution's audit row is the window's newest, and it diverged.
    let recent = input.recent_divergences + 1;
    let verdict = Verdict {
        reason: input.reason.clone(),
        compared_with: Some(input.hit_run.to_string()),
        divergence: Some(divergence),
        byte_identical: false,
        streak: 0,
        needed: needed(policy, recent),
        trust: trust.clone(),
        would_have_saved: None,
        unprovable: None,
        suspects: 0,
        shadow_left: Schedule::of(policy, recent).shadow_left(0),
        repaid: false,
        superseded: 0,
    };
    DivergenceTransition {
        trust,
        audit,
        verdict,
    }
}

/// A served replay's effect on the key's trust row: one more would-be hit,
/// one more replay. Pure.
pub fn replay_transition(trust: Option<TrustRow>, key: Hash, cmd: &str) -> TrustRow {
    let mut t = trust.unwrap_or_else(|| TrustRow::new(key, Some(cmd)));
    t.eligible += 1;
    t.replays += 1;
    t
}

// ----- the shell: the transitions over a ledger and a blob store -----

/// The trust layer over a ledger and a blob store: it reads the facts a
/// transition needs, calls the transition, and writes what it returns, in
/// one `atomically`. Generic so that quod-server runs exactly this code
/// over its own stores; every local path uses the defaults (SQLite, the
/// local CAS).
pub struct Trust<'a, L: LedgerStore = Ledger, B: BlobStore + ?Sized = Cas> {
    ledger: &'a L,
    cas: &'a B,
    pub policy: Policy,
}

impl<'a, L: LedgerStore, B: BlobStore + ?Sized> Trust<'a, L, B> {
    pub fn new(ledger: &'a L, cas: &'a B, policy: Policy) -> Result<Trust<'a, L, B>, TrustError> {
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
        decide_in(self.ledger, self.cas, &self.policy, key, hit)
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
    /// caller must execute after all, for that reason; `Deferred` means the
    /// caller owes a deferred audit and must enqueue it in this transaction.
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
                l.backlog(&step.key, &stale_cutoff())?,
            );
            let deferred = match decision {
                Decision::Execute(reason) => return Ok(Replayed::Execute(reason)),
                Decision::Defer { nth } => Some(nth),
                Decision::Replay => None,
            };
            l.put_trust(&replay_transition(trust, step.key, step.cmd))?;
            let facts = self.facts(&step.key, &mid)?;
            Ok(match deferred {
                Some(nth) => Replayed::Deferred(facts, nth),
                None => Replayed::Replayed(facts),
            })
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
    /// if that is another run; `execution_transition` decides what that
    /// means for the key.
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
            let trust = l.trust(&key)?;
            let prior = l.proof(&key, &manifest)?;

            let mut c = Compared {
                byte_identical: true,
                ..Compared::default()
            };
            // What a replay would have served. The tree matched its
            // manifest; the step must read the same inputs (up to absent
            // probes a recorded listing already pins) and produce the same
            // result.
            if let (Some((hm, hr)), Some(hid)) = (hit, hit_manifest) {
                if hid != manifest
                    && let Err(at) = equivalent(hm, executed)
                {
                    c.divergence.push(format!(
                        "inputs differed: the tree matched manifest b3:{} but this execution read manifest b3:{} ({at})",
                        hid.short(),
                        manifest.short()
                    ));
                }
                c.divergence
                    .merge(compare(hr, result, self.cas, step.ignore)?);
                c.byte_identical &= same_bytes(hr, result, step.ignore);
                c.compared_with = Some(hr.run_id.clone());
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
            if let Some(p) = &prior {
                c.previous = Some(if c.compared_with.as_deref() == Some(p.last_run.as_str()) {
                    let (_, hr) = hit.expect("compared_with is the hit's run");
                    Some(fingerprint(hr, self.cas, step.ignore)?)
                } else {
                    match self.result_of_run(&p.last_run, &key)? {
                        Some(prev) => {
                            c.compared_with.get_or_insert_with(|| p.last_run.clone());
                            c.divergence
                                .merge(compare(&prev, result, self.cas, step.ignore)?);
                            c.byte_identical &= same_bytes(&prev, result, step.ignore);
                            Some(fingerprint(&prev, self.cas, step.ignore)?)
                        }
                        None => None,
                    }
                });
            }
            // This execution's audit row is the window's newest.
            let recent = l.recent_divergences(&key, TRAILING_AUDITS.saturating_sub(1))?;
            let t = execution_transition(
                &self.policy,
                ExecutionInput {
                    key,
                    cmd: step.cmd,
                    run_id,
                    manifest,
                    fingerprint: fp,
                    unprovable: why_unprovable.clone(),
                    reason,
                    trust,
                    prior,
                    compared: c,
                    recent_divergences: recent,
                    hit_duration_ms: hit.map(|(_, r)| r.duration_ms),
                    now: now_rfc3339(),
                },
            );
            if t.reset_proofs {
                l.reset_proofs(&key)?;
                if let Some(divergence) = &t.verdict.divergence {
                    tracing::warn!(target: "quod", step_key = %key, manifest_id = %manifest, divergence = %divergence, "divergence: step key quarantined");
                }
            }
            l.put_proof(&t.proof)?;
            l.put_trust(&t.trust)?;
            let mut verdict = t.verdict;
            if let Some(a) = &t.audit {
                let audit_id = l.record_audit(a)?;
                if t.mark_suspects
                    && let Some(divergence) = &verdict.divergence
                {
                    verdict.suspects = mark_suspects(l, &key, &t.trust, divergence, audit_id)?;
                    verdict.superseded = supersede_pending(l, &key, &t.trust, reason, audit_id)?;
                }
                match reason {
                    // The deferred audit is settled by its verdict, in the
                    // transaction that writes it.
                    Reason::DeferredAudit { pending, .. } => {
                        let none = Divergence::default();
                        l.finish_audit(
                            *pending,
                            verdict.divergence.is_some(),
                            audit_id,
                            &truncate(
                                &verdict_detail(
                                    verdict.divergence.as_ref().unwrap_or(&none),
                                    verdict.byte_identical,
                                ),
                                300,
                            ),
                        )?;
                    }
                    // An inline audit makes up for a lost or overdue
                    // deferred one, if any was owed.
                    Reason::Audit { .. } => {
                        verdict.repaid = l.repay_audit(&key, audit_id, &stale_cutoff())?;
                    }
                    _ => {}
                }
            }
            Ok(verdict)
        })
    }

    /// Records an execution of a hit that produced no result to compare,
    /// yet behaved otherwise than the run a replay would serve: the runner
    /// hooks run a keyed step offline, and on the same inputs it failed,
    /// reached for the network, or wrote where the recorded run did not
    /// (`why` says how). That is a divergence like any other: the key is
    /// quarantined, every manifest of it proves again, the audit row says
    /// what happened, and the key's earlier replays become suspect. A run
    /// that was the machine's (killed, out of memory) is not one: that is
    /// `record_incomplete`.
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
            let t = divergence_transition(
                &self.policy,
                DivergenceInput {
                    key,
                    cmd: step.cmd,
                    run_id,
                    manifest,
                    hit_run: &hr.run_id,
                    reason,
                    why,
                    trust: l.trust(&key)?,
                    recent_divergences: l
                        .recent_divergences(&key, TRAILING_AUDITS.saturating_sub(1))?,
                    now: now_rfc3339(),
                },
            );
            l.reset_proofs(&key)?;
            l.put_trust(&t.trust)?;
            let audit_id = l.record_audit(&t.audit)?;
            let mut verdict = t.verdict;
            let divergence = verdict.divergence.clone().expect("a divergence");
            verdict.suspects = mark_suspects(l, &key, &t.trust, &divergence, audit_id)?;
            verdict.superseded = supersede_pending(l, &key, &t.trust, reason, audit_id)?;
            tracing::warn!(target: "quod", step_key = %key, manifest_id = %manifest, divergence = %divergence, "divergence without a result: step key quarantined");
            Ok(verdict)
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

/// Trust v2, at a quarantine: labels every replay the key has served since
/// its last quarantine as suspect, with when and why (docs/trust-v2.md,
/// "Retroactive marking"). Returns how many were newly labelled.
fn mark_suspects<L: LedgerStore>(
    l: &L,
    key: &Hash,
    trust: &TrustRow,
    divergence: &Divergence,
    audit_id: i64,
) -> Result<usize, TrustError> {
    let why = format!(
        "{}: key quarantined after a divergence: {} (audit {audit_id})",
        trust.quarantined_at.as_deref().unwrap_or(""),
        truncate(&divergence.to_string(), 200)
    );
    let n = l.mark_suspect_replays(key, &why)?;
    if n > 0 {
        tracing::warn!(target: "quod", step_key = %key, suspects = n, "replays marked suspect: they may have served a wrong result");
    }
    Ok(n)
}

/// At a quarantine: the key's other deferred audits, still outstanding,
/// would compare against the served result the quarantine has just
/// declared suspect, and count the one divergence again (and mark the
/// replays a re-proof later serves). Dropped as superseded, repaid by the
/// restarted shadow. The row the quarantining execution settles itself is
/// left to it. Returns how many were dropped.
fn supersede_pending<L: LedgerStore>(
    l: &L,
    key: &Hash,
    trust: &TrustRow,
    reason: &Reason,
    audit_id: i64,
) -> Result<usize, TrustError> {
    let except = match reason {
        Reason::DeferredAudit { pending, .. } => Some(*pending),
        _ => None,
    };
    let why = format!(
        "superseded by the quarantine at {} (audit {audit_id}): it would have compared against a result now suspect",
        trust.quarantined_at.as_deref().unwrap_or("")
    );
    let n = l.supersede_pending(key, except, &why)?;
    if n > 0 {
        tracing::warn!(target: "quod", step_key = %key, superseded = n, "deferred audits dropped at a quarantine");
    }
    Ok(n)
}

/// One line for a pending audit's settlement.
fn verdict_detail(divergence: &Divergence, byte_identical: bool) -> String {
    if !divergence.is_empty() {
        divergence.to_string()
    } else if byte_identical {
        "byte-identical".to_string()
    } else {
        "identical up to normalized volatile fields".to_string()
    }
}

/// `Trust::decide` on any stores: the stored facts, then `decide`.
fn decide_in<L: LedgerStore, B: BlobStore + ?Sized>(
    ledger: &L,
    cas: &B,
    policy: &Policy,
    key: &Hash,
    hit: Option<&(Manifest, StepResult)>,
) -> Result<Decision, TrustError> {
    let trust = ledger.trust(key)?;
    let (proof, why) = match hit {
        Some((m, r)) => (
            ledger.proof(key, &manifest_id(m)?)?,
            unprovable(policy, r, cas)?,
        ),
        None => (None, None),
    };
    Ok(decide(
        policy,
        hit.map(|(_, r)| HitView {
            run_id: &r.run_id,
            unprovable: why.as_deref(),
        }),
        trust.as_ref(),
        proof.as_ref(),
        ledger.recent_divergences(key, TRAILING_AUDITS)?,
        ledger.backlog(key, &stale_cutoff())?,
    ))
}

// ----- the step ledger: one interface for local and remote -----

/// What `StepLedger::claim` concluded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum Claimed {
    /// The hit replays; its run row is recorded. The receipt's trust facts,
    /// and the deferred audit this replay owes, if the decision deferred
    /// one (enqueued in the same transaction). Only a local ledger with a
    /// caller that can store a deferred audit ever sets it: quod-server
    /// never defers (remote runners audit inline; docs/cloud-ledger.md,
    /// "Deferred audits and remote runners"), and a client treats a claim
    /// answer that names one as a bad answer.
    Replay {
        facts: Facts,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        deferred: Option<DeferredAudit>,
    },
    /// Execute, for this reason (the lookup's hit, if any, is what the
    /// execution is compared with).
    Execute { reason: Reason },
}

/// A deferred audit a replay owes: would-be hit `nth`, pending audit
/// `pending` in the ledger that served it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeferredAudit {
    pub nth: u64,
    pub pending: i64,
}

/// What a deferred audit stores beside its pending row, made by the caller
/// only when the decision defers: the exact input tree (a document in the
/// store) and the request the audit worker executes.
#[derive(Clone, Debug)]
pub struct DeferInput {
    pub tree: Hash,
    pub request: Vec<u8>,
}

/// The caller's side of a deferral: given the would-be hit's number, the
/// tree and request to store with the pending audit. Called inside the
/// claim's transaction; an error rolls the replay back.
pub type Defer<'a> = &'a mut dyn FnMut(u64) -> Result<DeferInput, TrustError>;

/// Everything `quod run` and the runner hooks ask of a ledger, at the
/// grain of a step: look up and decide, then commit what happened. Two
/// implementations: `LocalLedger` (a SQLite ledger and the local CAS, the
/// path every existing test exercises) and quod-wire's `RemoteLedger`
/// (quod-server over quod.v1, where the server makes the same calls on its
/// own stores, inside its own transactions).
///
/// Run rows travel as `RunRecord`s. `commit_execution` and
/// `commit_divergence` fill the record's `outcome` (and, for an execution,
/// `saved_ms`) from the verdict, as the callers did inline before.
pub trait StepLedger {
    /// Records the canonical JSON a step key was computed from.
    fn put_step_key(&self, key: &Hash, json: &str) -> Result<(), TrustError>;

    /// Looks `step` up against `tree`, decides, and on a replay records the
    /// replay (trust counters, and the run row `run` builds from the result
    /// being replayed) in one transaction. `defer` is how the caller stores
    /// a deferred audit; a caller without one must hold a policy that never
    /// defers (`Policy::defer` off), which `LocalLedger` checks.
    fn claim(
        &self,
        step: &Step,
        tree: &Tree,
        run: &mut dyn FnMut(&StepResult) -> RunRecord,
        defer: Option<Defer<'_>>,
    ) -> Result<(Lookup, Claimed), TrustError>;

    /// `Trust::record_execution` and the run row, manifest and result, in
    /// one transaction.
    #[allow(clippy::too_many_arguments)]
    fn commit_execution(
        &self,
        step: &Step,
        run_id: &str,
        manifest: &Manifest,
        result: &StepResult,
        hit: Option<&(Manifest, StepResult)>,
        reason: &Reason,
        run: RunRecord,
    ) -> Result<Verdict, TrustError>;

    /// `Trust::record_divergence` and the run row, in one transaction.
    #[allow(clippy::too_many_arguments)]
    fn commit_divergence(
        &self,
        step: &Step,
        run_id: &str,
        hit: &(Manifest, StepResult),
        reason: &Reason,
        why: &str,
        run: RunRecord,
    ) -> Result<Verdict, TrustError>;

    /// `Trust::record_incomplete` and, if given, the run row, in one
    /// transaction.
    fn commit_incomplete(
        &self,
        step: &Step,
        hit: Option<&(Manifest, StepResult)>,
        what: &str,
        run: Option<RunRecord>,
    ) -> Result<Option<String>, TrustError>;

    /// A run row on its own (a killed or tainted run: nothing learned).
    fn record_run(&self, run: RunRecord) -> Result<(), TrustError>;
}

/// The step ledger over a local ledger and the trust layer: exactly the
/// calls `quod run` and the hooks made before the interface existed, in the
/// same order and the same transactions.
pub struct LocalLedger<'a, L: LedgerStore = Ledger, B: BlobStore + ?Sized = Cas> {
    pub ledger: &'a L,
    pub trust: &'a Trust<'a, L, B>,
}

impl<L: LedgerStore, B: BlobStore + ?Sized> StepLedger for LocalLedger<'_, L, B> {
    fn put_step_key(&self, key: &Hash, json: &str) -> Result<(), TrustError> {
        Ok(self.ledger.put_step_key(key, json)?)
    }

    fn claim(
        &self,
        step: &Step,
        tree: &Tree,
        run: &mut dyn FnMut(&StepResult) -> RunRecord,
        mut defer: Option<Defer<'_>>,
    ) -> Result<(Lookup, Claimed), TrustError> {
        if self.trust.policy.defer && defer.is_none() {
            // A deferral this caller cannot store would serve a replay with
            // its audit lost at birth.
            return Err(TrustError::BadPolicy(
                "this caller audits inline: its policy must not defer audits".into(),
            ));
        }
        let lookup = self.ledger.find_match(&step.key, tree)?;
        let claimed = match self.trust.decide(&step.key, lookup.hit.as_ref())? {
            Decision::Replay | Decision::Defer { .. } => {
                let hit = lookup.hit.as_ref().expect("a replay needs a hit");
                // Decided again as it is recorded (another run may have
                // quarantined the key or taken the audit slot meanwhile);
                // a deferral is enqueued on what that says, in the same
                // transaction as the replay and its run row.
                self.ledger.atomically(|l| -> Result<_, TrustError> {
                    Ok(match self.trust.record_replay(step, hit)? {
                        Replayed::Replayed(facts) => {
                            l.record_run_facts(&run(&hit.1).facts())?;
                            Claimed::Replay {
                                facts,
                                deferred: None,
                            }
                        }
                        Replayed::Deferred(facts, nth) => {
                            let record = run(&hit.1);
                            l.record_run_facts(&record.facts())?;
                            let defer = defer
                                .as_mut()
                                .expect("a deferring policy has a caller that stores the audit");
                            let input = defer(nth)?;
                            let pending = l.enqueue_audit(&NewPending {
                                step_key: &step.key,
                                manifest_id: &hit.1.manifest_id,
                                hit_run: &hit.1.run_id,
                                replay_run: &record.id,
                                nth,
                                tree: &input.tree,
                                request: &input.request,
                            })?;
                            tracing::info!(target: "quod", pending, step_key = %step.key, manifest_id = %hit.1.manifest_id, nth, replay_run = %record.id, "audit deferred");
                            Claimed::Replay {
                                facts,
                                deferred: Some(DeferredAudit { nth, pending }),
                            }
                        }
                        Replayed::Execute(reason) => Claimed::Execute { reason },
                    })
                })?
            }
            Decision::Execute(reason) => Claimed::Execute { reason },
        };
        Ok((lookup, claimed))
    }

    fn commit_execution(
        &self,
        step: &Step,
        run_id: &str,
        manifest: &Manifest,
        result: &StepResult,
        hit: Option<&(Manifest, StepResult)>,
        reason: &Reason,
        mut run: RunRecord,
    ) -> Result<Verdict, TrustError> {
        self.ledger.atomically(|l| -> Result<Verdict, TrustError> {
            let v = self
                .trust
                .record_execution(step, run_id, manifest, result, hit, reason)?;
            run.outcome = Some(v.outcome().to_string());
            run.saved_ms = v.would_have_saved.map(|d| d.as_millis() as u64);
            l.record_execution_facts(&run.facts(), manifest, result)?;
            Ok(v)
        })
    }

    fn commit_divergence(
        &self,
        step: &Step,
        run_id: &str,
        hit: &(Manifest, StepResult),
        reason: &Reason,
        why: &str,
        mut run: RunRecord,
    ) -> Result<Verdict, TrustError> {
        self.ledger.atomically(|l| -> Result<Verdict, TrustError> {
            let v = self
                .trust
                .record_divergence(step, run_id, hit, reason, why)?;
            run.outcome = Some(v.outcome().to_string());
            l.record_run_facts(&run.facts())?;
            Ok(v)
        })
    }

    fn commit_incomplete(
        &self,
        step: &Step,
        hit: Option<&(Manifest, StepResult)>,
        what: &str,
        run: Option<RunRecord>,
    ) -> Result<Option<String>, TrustError> {
        self.ledger.atomically(|l| -> Result<_, TrustError> {
            let line = self.trust.record_incomplete(step, hit, what)?;
            if let Some(run) = &run {
                l.record_run_facts(&run.facts())?;
            }
            Ok(line)
        })
    }

    fn record_run(&self, run: RunRecord) -> Result<(), TrustError> {
        Ok(self.ledger.record_run_facts(&run.facts())?)
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

    /// v1's policy (two proofs, a flat 5% audit): the behavioural tests
    /// below keep their meaning under it; trust v2's defaults have their own.
    const V1: Policy = Policy {
        mode: Mode::Observe,
        proofs: 2,
        shadow: 0,
        audit: 0.05,
        replay_failures: false,
        defer: false,
    };

    #[test]
    fn decisions() {
        let replay = Policy {
            mode: Mode::Replay,
            ..V1
        };
        let observe = V1;
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
                Backlog::default(),
            )
        };
        assert_eq!(
            d(&replay, false, None, None),
            Decision::Execute(Reason::Miss)
        );
        // A streak vouches only for the result of the run it ends with: a
        // hit serving another run's result is unproven, however long.
        assert_eq!(
            decide(
                &replay,
                Some(view("01OTHER")),
                None,
                Some(&proof(9)),
                0,
                Backlog::default()
            ),
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
                0,
                Backlog::default()
            ),
            Decision::Execute(Reason::Unprovable {
                why: "exit code 1".into()
            })
        );
        // The 20th eligible hit is audited at 5%.
        let t19 = trust_row(TrustState::Proven, 19, 0);
        assert_eq!(
            d(&replay, true, Some(&t19), Some(&proof(5))),
            Decision::Execute(Reason::Audit {
                nth: 20,
                shadow: false
            })
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

    /// Trust v2: ISO 8601 timestamps with a time of day and a test runner's
    /// durations (in parentheses, or after in/took/Time/Duration/Finished)
    /// are erased from stdout and stderr; counts, names, bare dates, compound
    /// Go durations and the kill markers stay.
    #[test]
    fn normalizes_timestamps_and_runner_durations() {
        let n = |s: &str| String::from_utf8(normalize(s.as_bytes())).unwrap();
        // Timestamps: T or space, fraction, Z or offset; a bare date stays.
        assert_eq!(
            n("2026-11-02T09:12:03Z INFO started\n"),
            "<ts> INFO started\n"
        );
        assert_eq!(
            n("[2026-11-02 09:12:03.007+01:00] ready; built 2026-11-02 for v2026-11-02\n"),
            "[<ts>] ready; built 2026-11-02 for v2026-11-02\n"
        );
        assert_eq!(
            n("id 12026-11-02T09:12:03Z\n"),
            "id 12026-11-02T09:12:03Z\n"
        );
        assert_eq!(n("2026-11-02T09:12:03Zx\n"), "2026-11-02T09:12:03Zx\n");
        // Durations in parentheses: mocha, vitest, cargo.
        assert_eq!(
            n("  ✓ adds (12ms)\n  ✓ subtracts (1.23 s)\n"),
            "  ✓ adds (<dur>)\n  ✓ subtracts (<dur>)\n"
        );
        // After a word: pytest, jest, cargo test, rspec, yarn.
        assert_eq!(
            n("===== 12 passed in 3.21s =====\n"),
            "===== 12 passed in <dur> =====\n"
        );
        assert_eq!(n("Time:        1.234 s\n"), "Time:        <dur>\n");
        assert_eq!(
            n("test result: ok. 3 passed; finished in 0.02s\n"),
            "test result: ok. 3 passed; finished in <dur>\n"
        );
        assert_eq!(
            n("Finished in 0.5 seconds (files took 1.2 seconds to load)\n"),
            "Finished in <dur> (files took <dur> to load)\n"
        );
        assert_eq!(n("   Duration  1.23s\n"), "   Duration  <dur>\n");
        assert_eq!(n("Done in 2.30s.\n"), "Done in <dur>.\n");
        // Not durations: a count, a word that merely ends in `in`, a unit
        // that is a prefix of another word, a compound Go duration.
        assert_eq!(n("in 3 modules\n"), "in 3 modules\n");
        assert_eq!(n("within 3s of start\n"), "within 3s of start\n");
        assert_eq!(
            n("2 passed in 3s; 1 passed in 3s\n"),
            "2 passed in <dur>; 1 passed in <dur>\n"
        );
        assert_eq!(n("inputs 12 files\n"), "inputs 12 files\n");
        assert_eq!(n("(1m30.5s)\n"), "(1m30.5s)\n");
        assert_eq!(
            n("panic: test timed out after 10m0s\n"),
            "panic: test timed out after 10m0s\n"
        );
        // The two outputs a flaky-looking runner prints fingerprint the same.
        let (a, b) = (
            n("[2026-11-02T09:12:03Z] 12 passed in 3.21s\n"),
            n("[2026-11-03T10:00:00Z] 12 passed in 2.98s\n"),
        );
        assert_eq!(a, b);
        assert_ne!(n("12 passed in 3.21s\n"), n("11 passed in 3.21s\n"));
    }

    /// fix/replay-gaps: every go test package-line form keeps only its
    /// duration out (expr-lang/expr's `[no tests to run]` lines diverged
    /// every audit), go test -json's Elapsed and escaped package lines,
    /// stdout and stderr alike.
    #[test]
    fn normalizes_every_go_test_status_line() {
        let n = |s: &str| String::from_utf8(normalize(s.as_bytes())).unwrap();
        // expr-lang/expr, go test ./... (go 1.27).
        let a = "ok  \tgithub.com/expr-lang/expr\t0.412s\n\
                 ok  \tgithub.com/expr-lang/expr/test/gen\t0.001s [no tests to run]\n\
                 ?   \tgithub.com/expr-lang/expr/internal/spew\t[no test files]\n\
                 ok  \tgithub.com/expr-lang/expr/test/fuzz\t(cached) [no tests to run]\n\
                 ok  \tgithub.com/expr-lang/expr/vm\t1.207s\tcoverage: 81.2% of statements\n\
                 ok  \tgithub.com/expr-lang/expr/test/mock\t0.004s\tcoverage: 0.0% of statements [no tests to run]\n\
                 ok  \tgithub.com/expr-lang/expr/types\t0.004s\tcoverage: [no statements]\n\
                 FAIL\tgithub.com/expr-lang/expr/vm\t0.031s\n\
                 FAIL\tgithub.com/expr-lang/expr/bad [build failed]\n";
        let b = "ok  \tgithub.com/expr-lang/expr\t0.398s\n\
                 ok  \tgithub.com/expr-lang/expr/test/gen\t0.002s [no tests to run]\n\
                 ?   \tgithub.com/expr-lang/expr/internal/spew\t[no test files]\n\
                 ok  \tgithub.com/expr-lang/expr/test/fuzz\t0.003s [no tests to run]\n\
                 ok  \tgithub.com/expr-lang/expr/vm\t(cached)\tcoverage: 81.2% of statements\n\
                 ok  \tgithub.com/expr-lang/expr/test/mock\t0.005s\tcoverage: 0.0% of statements [no tests to run]\n\
                 ok  \tgithub.com/expr-lang/expr/types\t0.006s\tcoverage: [no statements]\n\
                 FAIL\tgithub.com/expr-lang/expr/vm\t0.044s\n\
                 FAIL\tgithub.com/expr-lang/expr/bad [build failed]\n";
        assert_eq!(n(a), n(b));
        assert_eq!(
            n("ok  \tgithub.com/expr-lang/expr/test/gen\t0.001s [no tests to run]\n"),
            "ok  \tgithub.com/expr-lang/expr/test/gen\t<dur> [no tests to run]\n"
        );
        assert_eq!(
            n("ok  \tpkg\t0.004s\tcoverage: 0.0% of statements [no tests to run]\n"),
            "ok  \tpkg\t<dur>\tcoverage: 0.0% of statements [no tests to run]\n"
        );
        // What the note says still counts: a package that ran tests is not
        // one that had none to run, and coverage numbers stay.
        assert_ne!(
            n("ok  \tpkg\t0.001s [no tests to run]\n"),
            n("ok  \tpkg\t0.001s\n")
        );
        assert_ne!(
            n("ok  \tpkg\t0.1s\tcoverage: 81.2% of statements\n"),
            n("ok  \tpkg\t0.1s\tcoverage: 81.3% of statements\n")
        );
        // A third field that does not start with a duration stays whole.
        assert_eq!(n("ok  \tpkg\t(stale) [x]\n"), "ok  \tpkg\t(stale) [x]\n");
        assert_eq!(n("ok  \tpkg\t1m0s [x]\n"), "ok  \tpkg\t1m0s [x]\n");
        // go test -v: RUN/PAUSE/CONT/NAME carry nothing; PASS/FAIL/SKIP
        // lines at any depth lose their duration.
        assert_eq!(
            n(
                "=== RUN   TestRun\n=== PAUSE TestRun\n=== CONT  TestRun\n=== NAME  TestRun\n    --- PASS: TestRun/sub_1 (0.00s)\n--- PASS: TestRun (0.01s)\nPASS\n"
            ),
            "=== RUN   TestRun\n=== PAUSE TestRun\n=== CONT  TestRun\n=== NAME  TestRun\n    --- PASS: TestRun/sub_1 (<dur>)\n--- PASS: TestRun (<dur>)\nPASS\n"
        );
        assert_eq!(
            n("testing: warning: no tests to run\nPASS\n"),
            "testing: warning: no tests to run\nPASS\n"
        );
        // go test -json (go 1.27): Time, Elapsed, and the lines in Output.
        let j1 = r#"{"Time":"2026-11-02T09:12:03.123456789Z","Action":"output","Package":"github.com/expr-lang/expr/test/gen","Output":"ok  \tgithub.com/expr-lang/expr/test/gen\t0.001s [no tests to run]\n"}
{"Time":"2026-11-02T09:12:03.123500000Z","Action":"pass","Package":"github.com/expr-lang/expr/test/gen","Elapsed":0.001}
{"Time":"2026-11-02T09:12:03.1Z","Action":"output","Package":"p","Test":"TestX","Output":"--- PASS: TestX (0.00s)\n"}
{"Time":"2026-11-02T09:12:03.1Z","Action":"pass","Package":"p","Test":"TestX","Elapsed":0}
"#;
        let j2 = r#"{"Time":"2026-11-03T10:00:00.5Z","Action":"output","Package":"github.com/expr-lang/expr/test/gen","Output":"ok  \tgithub.com/expr-lang/expr/test/gen\t0.012s [no tests to run]\n"}
{"Time":"2026-11-03T10:00:00.6Z","Action":"pass","Package":"github.com/expr-lang/expr/test/gen","Elapsed":0.012}
{"Time":"2026-11-03T10:00:00.7Z","Action":"output","Package":"p","Test":"TestX","Output":"--- PASS: TestX (0.31s)\n"}
{"Time":"2026-11-03T10:00:00.8Z","Action":"pass","Package":"p","Test":"TestX","Elapsed":0.31}
"#;
        assert_eq!(n(j1), n(j2));
        assert_ne!(
            n(r#"{"Time":"2026-11-02T09:12:03Z","Action":"pass","Package":"p","Elapsed":1}"#),
            n(r#"{"Time":"2026-11-02T09:12:03Z","Action":"fail","Package":"p","Elapsed":1}"#)
        );
        // Not a go test event: an Elapsed elsewhere stays.
        assert_eq!(n(r#"{"Elapsed":0.5}"#), r#"{"Elapsed":0.5}"#);
    }

    /// fix/replay-gaps: node:test's unitless duration_ms (spec and TAP
    /// reporters; commander.js and bench-lite's node-test diverged on every
    /// audit), and the timing fields jest, vitest, pytest and mocha print
    /// that the generic rules did not reach.
    #[test]
    fn normalizes_node_test_and_other_runner_timings() {
        let n = |s: &str| String::from_utf8(normalize(s.as_bytes())).unwrap();
        // node --test, spec reporter (node 24, commander.js).
        let spec = |d1: &str, d2: &str, total: &str| {
            format!(
                "▶ .addHelpText\n  ✔ when \"before\" string then string before built-in help ({d1}ms)\n  ✔ when \"after\" string then string after built-in help ({d2}ms)\n▶ .addHelpText ({d2}ms)\nℹ tests 1243\nℹ suites 412\nℹ pass 1243\nℹ fail 0\nℹ cancelled 0\nℹ skipped 0\nℹ todo 0\nℹ duration_ms {total}\n"
            )
        };
        assert_eq!(
            n(&spec("0.9113", "0.2", "1541.8")),
            n(&spec("1.0402", "0.31", "1297.412"))
        );
        assert!(n(&spec("1", "2", "1541.8")).ends_with("ℹ duration_ms <dur>\n"));
        // The counts stay.
        assert_ne!(
            n("ℹ pass 1243\nℹ duration_ms 1.5\n"),
            n("ℹ pass 1242\nℹ duration_ms 1.5\n")
        );
        // node --test, TAP reporter.
        let tap = |d: &str, total: &str| {
            format!(
                "TAP version 13\n# Subtest: adds\nok 1 - adds\n  ---\n  duration_ms: {d}\n  type: 'test'\n  ...\n1..1\n# tests 1\n# suites 0\n# pass 1\n# fail 0\n# duration_ms {total}\n"
            )
        };
        assert_eq!(n(&tap("0.4567", "61.2")), n(&tap("12", "70.03")));
        assert!(n(&tap("1", "2")).contains("  duration_ms: <dur>\n"));
        // Not a duration: another word, no number, a version-like token.
        assert_eq!(n("max_duration_ms 5\n"), "max_duration_ms 5\n");
        assert_eq!(n("duration_ms: unknown\n"), "duration_ms: unknown\n");
        assert_eq!(n("duration_ms 1.2.3\n"), "duration_ms 1.2.3\n");
        assert_eq!(n("duration_ms 12abc\n"), "duration_ms 12abc\n");
        // jest: the estimate after Time.
        assert_eq!(
            n("Time:        1.234 s, estimated 2 s\n"),
            "Time:        <dur>, estimated <dur>\n"
        );
        assert_eq!(
            n("PASS src/sum.test.js (5.123 s)\n"),
            "PASS src/sum.test.js (<dur>)\n"
        );
        // vitest: start clock, the Duration breakdown, a file's line.
        assert_eq!(n(" Start at  09:12:03\n"), " Start at  <ts>\n");
        assert_eq!(
            n(
                "   Duration  1.23s (transform 45ms, setup 0ms, collect 100ms, tests 20ms, environment 0ms, prepare 50ms)\n"
            ),
            "   Duration  <dur> (transform <dur>, setup <dur>, collect <dur>, tests <dur>, environment <dur>, prepare <dur>)\n"
        );
        assert_eq!(
            n(
                " ✓ src/a.test.ts (3 tests) 5ms\n ✓ src/b.test.ts (4 tests | 1 skipped) 125ms\n ✓ src/c.test.ts (1 test) 2ms\n"
            ),
            " ✓ src/a.test.ts (3 tests) <dur>\n ✓ src/b.test.ts (4 tests | 1 skipped) <dur>\n ✓ src/c.test.ts (1 test) <dur>\n"
        );
        assert_eq!(
            n("      Tests  3 passed (3)\n"),
            "      Tests  3 passed (3)\n"
        );
        assert_eq!(
            n(" ✓ src/a.test.ts (3 items) 5ms\n"),
            " ✓ src/a.test.ts (3 items) 5ms\n"
        );
        assert_eq!(n("Start at noon\n"), "Start at noon\n");
        // pytest: the clock after a long run's duration.
        assert_eq!(
            n("======= 1203 passed, 4 skipped in 62.01s (0:01:02) =======\n"),
            "======= 1203 passed, 4 skipped in <dur> (<dur>) =======\n"
        );
        assert_eq!(
            n("in 3.2s (version 1:02:03)\n"),
            "in <dur> (version 1:02:03)\n"
        );
        // mocha.
        assert_eq!(
            n("  12 passing (12ms)\n  1 pending\n"),
            "  12 passing (<dur>)\n  1 pending\n"
        );
    }

    /// Parallel runners' completion order is not compared; which results
    /// there were always is.
    #[test]
    fn parallel_runner_order_is_canonical_and_results_still_count() {
        let n = |s: &str| String::from_utf8(normalize(s.as_bytes())).unwrap();
        let head = "============================= test session starts ==============================\nplatform linux -- Python 3.12.16, pytest-8.4.2, pluggy-1.6.0\nrootdir: /workspace\nplugins: xdist-3.8.0\ncreated: 4/4 workers\n4 workers [12 items]\n\n";
        // pytest -n 4 (Moto): the skips land on other lines run to run.
        let a = format!(
            "{head}..s.......s. [ 50%]\n..........s. [100%]\n\n======================== 9 passed, 3 skipped in 1.21s =========================\n"
        );
        let b = format!(
            "{head}s.........s. [ 50%]\n.s.......... [100%]\n\n======================== 9 passed, 3 skipped in 1.40s =========================\n"
        );
        assert_eq!(n(&a), n(&b));
        // A pass that became a failure is a different result.
        let c = format!(
            "{head}s.........s. [ 50%]\n.s......F... [100%]\n\n=================== 1 failed, 8 passed, 3 skipped in 1.40s ====================\n"
        );
        assert_ne!(n(&a), n(&c));
        // -q: xdist's only mark is `bringing up nodes...`.
        assert_eq!(
            n("bringing up nodes...\n\n..s. [ 50%]\n.... [100%]\n"),
            n("bringing up nodes...\n\n.... [ 50%]\n.s.. [100%]\n")
        );
        // Even with the summary line removed, the progress counts differ.
        let strip = |s: &str| {
            s.lines()
                .filter(|l| !l.contains("passed"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert_ne!(strip(&n(&a)), strip(&n(&c)));
        // Without xdist's header the order is compared as printed (a
        // sequential runner's order can be a result).
        assert_ne!(
            n("..s. [ 50%]\n.... [100%]\n"),
            n(".s.. [ 50%]\n.... [100%]\n")
        );
        // pytest -n 4 -v: which test had which result counts, not which
        // worker ran it or when.
        let v = |lines: &[&str]| {
            format!(
                "{head}[gw1] linux -- Python 3.12.16 /venv/bin/python\n[gw0] linux -- Python 3.12.16 /venv/bin/python\n{}\n",
                lines.join("\n")
            )
        };
        let va = v(&[
            "[gw0] [ 33%] PASSED tests/test_a.py::test_x ",
            "[gw1] [ 66%] SKIPPED tests/test_b.py::test_y ",
            "[gw0] [100%] PASSED tests/test_a.py::test_z ",
        ]);
        let vb = v(&[
            "[gw1] [ 33%] SKIPPED tests/test_b.py::test_y ",
            "[gw1] [ 66%] PASSED tests/test_a.py::test_z ",
            "[gw0] [100%] PASSED tests/test_a.py::test_x ",
        ]);
        assert_eq!(n(&va), n(&vb));
        assert!(n(&va).contains("[gw*] [ 33%] PASSED tests/test_a.py::test_x \n"));
        // The same counts, but another test skipped: different.
        let vc = v(&[
            "[gw1] [ 33%] PASSED tests/test_b.py::test_y ",
            "[gw1] [ 66%] SKIPPED tests/test_a.py::test_z ",
            "[gw0] [100%] PASSED tests/test_a.py::test_x ",
        ]);
        assert_ne!(n(&va), n(&vc));
        // One test passing in one run and failing in the other.
        let vd = v(&[
            "[gw1] [ 33%] SKIPPED tests/test_b.py::test_y ",
            "[gw1] [ 66%] FAILED tests/test_a.py::test_z ",
            "[gw0] [100%] PASSED tests/test_a.py::test_x ",
        ]);
        assert_ne!(n(&va), n(&vd));
        // -v without a terminal: start lines interleaved (attrs, -n 4 -v).
        let ia = v(&[
            "tests/test_a.py::test_x ",
            "tests/test_b.py::test_y ",
            "[gw0] [ 50%] PASSED tests/test_a.py::test_x ",
            "[gw1] [100%] PASSED tests/test_b.py::test_y ",
        ]);
        let ib = v(&[
            "tests/test_b.py::test_y ",
            "[gw1] [ 50%] PASSED tests/test_b.py::test_y ",
            "tests/test_a.py::test_x ",
            "[gw0] [100%] PASSED tests/test_a.py::test_x ",
        ]);
        let ic = v(&[
            "tests/test_b.py::test_y ",
            "[gw1] [ 50%] FAILED tests/test_b.py::test_y ",
            "tests/test_a.py::test_x ",
            "[gw0] [100%] PASSED tests/test_a.py::test_x ",
        ]);
        assert_eq!(n(&ia), n(&ib));
        assert_ne!(n(&ia), n(&ic));
        // The short test summary arrives in report order (attrs, -n 4).
        let s = |lines: &[&str]| {
            format!(
                "{head}... [100%]\n=========================== short test summary info ============================\n{}\n================= 1401 passed, 3 skipped in 8.68s ==================\n",
                lines.join("\n")
            )
        };
        let sa = s(&[
            "SKIPPED [1] tests/test_pyright.py:35: Requires pyright.",
            "SKIPPED [1] tests/test_pyright.py:89: Requires pyright.",
            "XFAIL tests/test_setattr.py::TestSetAttr::test_slotted_confused",
        ]);
        let sb = s(&[
            "SKIPPED [1] tests/test_pyright.py:89: Requires pyright.",
            "SKIPPED [1] tests/test_pyright.py:35: Requires pyright.",
            "XFAIL tests/test_setattr.py::TestSetAttr::test_slotted_confused",
        ]);
        let sc = s(&[
            "SKIPPED [1] tests/test_pyright.py:89: Requires pyright.",
            "FAILED tests/test_pyright.py::test_35 - assert 1",
            "XFAIL tests/test_setattr.py::TestSetAttr::test_slotted_confused",
        ]);
        assert_eq!(n(&sa), n(&sb));
        assert_ne!(n(&sa), n(&sc));
        // Outside the summary section, such a line keeps its place.
        assert_ne!(
            n(&format!("{head}SKIPPED a\nSKIPPED b\n")),
            n(&format!("{head}SKIPPED b\nSKIPPED a\n"))
        );
        // jest with workers: suites in completion order.
        let ja = "PASS src/a.test.js\nPASS src/b.test.js\n\nTest Suites: 2 passed, 2 total\nTests:       5 passed, 5 total\n";
        let jb = "PASS src/b.test.js\nPASS src/a.test.js\n\nTest Suites: 2 passed, 2 total\nTests:       5 passed, 5 total\n";
        let jc = "FAIL src/b.test.js\nPASS src/a.test.js\n\nTest Suites: 2 passed, 2 total\nTests:       5 passed, 5 total\n";
        assert_eq!(n(ja), n(jb));
        assert_ne!(n(ja), n(jc));
        assert_ne!(
            n("PASS src/a.test.js\nPASS src/b.test.js\n"),
            n("PASS src/b.test.js\nPASS src/a.test.js\n")
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

        /// Another tree for the same key (another commit): from now on
        /// `run` reads and records this manifest.
        fn switch_manifest(&mut self, gomod: &[u8]) {
            let tree = Tree::from_leaves([Entry::file("go.mod", Hash::of(gomod), 0o644)]);
            self.manifest = Manifest::from_log(
                self.key,
                vec!["/workspace".into()],
                &tree,
                [Access {
                    path: "go.mod".into(),
                    kind: Kind::File,
                }],
            )
            .unwrap();
            self.mid = manifest_id(&self.manifest).unwrap();
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
                Decision::Replay | Decision::Defer { .. } => {
                    let step = Step {
                        key: self.key,
                        cmd: "go test ./...",
                        ignore: &[],
                    };
                    let (m, r) = hit.as_ref().unwrap();
                    match trust.record_replay(&step, hit.as_ref().unwrap()).unwrap() {
                        replayed @ (Replayed::Replayed(_) | Replayed::Deferred(..)) => {
                            // The run row a replay writes, as the CLI and the
                            // hooks do: what a quarantine marks suspect.
                            self.runs += 1;
                            let run_id = format!("01RUN{:02}", self.runs);
                            self.ledger
                                .record_run_facts(&quod_ledger::RunFacts {
                                    id: &run_id,
                                    step_key: Some(&self.key),
                                    outcome: Some("replayed"),
                                    ..quod_ledger::RunFacts::default()
                                })
                                .unwrap();
                            // Decided again in the transaction, as the
                            // callers do: a deferral is enqueued on what
                            // that said, never on the first decision.
                            if let Replayed::Deferred(_, nth) = replayed {
                                self.ledger
                                    .enqueue_audit(&quod_ledger::NewPending {
                                        step_key: &self.key,
                                        manifest_id: &manifest_id(m).unwrap(),
                                        hit_run: &r.run_id,
                                        replay_run: &run_id,
                                        nth,
                                        tree: &Hash::of(b"tree"),
                                        request: b"{}",
                                    })
                                    .unwrap();
                                return Err(Decision::Defer { nth });
                            }
                            return Err(Decision::Replay);
                        }
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

        /// The audit worker: claims the oldest pending audit of the key
        /// and executes it with the given outcome, as `quod audit` does.
        fn audit(&mut self, policy: &Policy, exit: i32, stdout: &[u8]) -> Verdict {
            let pending = self
                .ledger
                .claim_audit("test")
                .unwrap()
                .expect("a pending audit");
            let manifest = self.ledger.manifest(&pending.manifest_id).unwrap().unwrap();
            let hr = self
                .ledger
                .results_for_run(&pending.hit_run)
                .unwrap()
                .into_iter()
                .find(|r| r.step_key == self.key)
                .unwrap();
            let trust = Trust::new(&self.ledger, &self.cas, *policy).unwrap();
            self.runs += 1;
            let r = World::mk(
                &self.cas,
                self.key,
                manifest_id(&manifest).unwrap(),
                self.runs,
                exit,
                stdout,
                vec![],
            );
            let v = trust
                .record_execution(
                    &Step {
                        key: self.key,
                        cmd: "go test ./...",
                        ignore: &[],
                    },
                    &r.run_id,
                    &manifest,
                    &r,
                    Some(&(manifest.clone(), hr)),
                    &Reason::DeferredAudit {
                        nth: pending.nth,
                        pending: pending.id,
                    },
                )
                .unwrap();
            self.ledger.put_result(&r).unwrap();
            v
        }
    }

    /// v1's replay policy: two proofs, a flat 5% audit, no shadow phase.
    const REPLAY: Policy = Policy {
        mode: Mode::Replay,
        ..V1
    };

    /// Trust v2's defaults in replay mode: one proof, a shadow of 2, 5%
    /// floor, with every audit inline (the schedule tests read as audits).
    const V2: Policy = Policy {
        mode: Mode::Replay,
        proofs: DEFAULT_PROOFS,
        shadow: DEFAULT_SHADOW,
        audit: DEFAULT_AUDIT,
        replay_failures: false,
        defer: false,
    };

    /// `V2` with the default deferral: decay audits serve the replay first.
    const V2D: Policy = Policy { defer: true, ..V2 };

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
            Replayed::Execute(Reason::Audit { nth: 2, .. })
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
        let observe = V1;
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
        assert_eq!(
            v.reason,
            Reason::Audit {
                nth: 1,
                shadow: false
            }
        );
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
        let reason = Reason::Audit {
            nth: 1,
            shadow: false,
        };
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
        let p = V1;
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
                Decision::Replay | Decision::Defer { .. } => Reason::WouldReplay,
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

    // ----- trust v2 (docs/trust-v2.md) -----

    /// The audit schedule: every hit in the shadow, a 1/k decay to the
    /// floor, deterministic; stricter after a divergence; v1's with no
    /// shadow; nothing at all at `--audit 0`.
    #[test]
    fn v2_schedule_shadows_then_decays_to_the_floor() {
        let due = |s: Schedule, n: u64| {
            (0..n)
                .filter(|k| s.due(*k))
                .map(|k| k + 1)
                .collect::<Vec<_>>()
        };
        let s = Schedule::of(&V2, 0);
        assert_eq!((s.shadow, s.floor), (2, 0.05));
        assert_eq!(due(s, 100), [1, 2, 4, 7, 11, 19, 31, 49, 69, 89]);
        assert_eq!(due(s, 200)[10..], [109, 129, 149, 169, 189]);
        assert_eq!(s.weight(1), 1.0);
        assert_eq!(s.weight(2), 1.0);
        assert!((s.weight(4) - 0.5).abs() < 1e-12);
        assert_eq!(s.weight(40), 0.05);
        assert_eq!(s.weight(1000), 0.05);
        assert!(s.in_shadow(0) && s.in_shadow(1) && !s.in_shadow(2));
        assert_eq!(
            (s.shadow_left(0), s.shadow_left(1), s.shadow_left(5)),
            (2, 1, 0)
        );
        // One recent divergence: shadow 4, floor 10%.
        let d1 = Schedule::of(&V2, 1);
        assert_eq!((d1.shadow, d1.floor), (4, 0.1));
        assert_eq!(due(d1, 40), [1, 2, 3, 4, 6, 7, 10, 12, 16, 20, 26, 33]);
        assert_eq!(due(d1, 80)[12..], [43, 53, 63, 73]);
        // Capped like the bar.
        let d9 = Schedule::of(&V2, 9);
        assert_eq!(d9.shadow, 2 * (1 + MAX_EXTRA_PROOFS));
        assert!((d9.floor - 0.3).abs() < 1e-12, "{}", d9.floor);
        // No shadow: v1's flat schedule; `audit_due` is that.
        let v1 = Schedule::of(&REPLAY, 0);
        assert_eq!(due(v1, 60), [20, 40, 60]);
        assert_eq!(
            (0..60).filter(|k| audit_due(*k, 0.05)).collect::<Vec<_>>(),
            [19, 39, 59]
        );
        // Audit 0: nothing, shadow included.
        let none = Schedule::of(&Policy { audit: 0.0, ..V2 }, 3);
        assert_eq!(
            none,
            Schedule {
                shadow: 0,
                floor: 0.0
            }
        );
        assert!(due(none, 100).is_empty());
        // Validation.
        assert!(V2.validate().is_ok());
        assert!(
            Policy {
                shadow: MAX_SHADOW + 1,
                ..V2
            }
            .validate()
            .is_err()
        );
    }

    /// Every execution trust v2's schedule asks for as evidence (shadow and
    /// decay audits, observe-mode would-be hits, proofs, re-proofs after a
    /// quarantine) says so, which is what makes `quod run --warm` expire
    /// go test's cache for it; only the record of a miss does not.
    #[test]
    fn every_audit_and_proof_is_evidence_and_a_miss_is_not() {
        let ok = b"ok  \tpkg\t0.4s\n";
        let mut w = World::new();
        let mut executed = vec![];
        for _ in 0..60 {
            if let Ok(v) = w.run(&V2, 0, ok) {
                executed.push(v.reason);
            }
        }
        assert_eq!(executed[0], Reason::Miss);
        assert!(!executed[0].is_evidence());
        assert!(
            executed[1..]
                .iter()
                .all(|r| matches!(r, Reason::Audit { .. })),
            "{executed:?}"
        );
        assert!(executed[1..].iter().all(Reason::is_evidence));
        assert!(
            executed.len() > 3,
            "the shadow and some decay: {executed:?}"
        );

        // A manifest still proving itself (--proofs 2), then observe mode.
        let mut w = World::new();
        let two = Policy { proofs: 2, ..V2 };
        assert_eq!(w.run(&two, 0, ok).unwrap().reason, Reason::Miss);
        let p = w.run(&two, 0, ok).unwrap().reason;
        assert!(
            matches!(p, Reason::Proving { .. }) && p.is_evidence(),
            "{p:?}"
        );
        let observe = Policy {
            mode: Mode::Observe,
            ..V2
        };
        let o = w.run(&observe, 0, ok).unwrap().reason;
        assert!(o == Reason::WouldReplay && o.is_evidence(), "{o:?}");

        // A divergence quarantines the key: its re-proof is evidence too.
        let d = w.run(&V2, 0, b"ok  \tpkg\t0.4s\nextra\n").unwrap();
        assert!(d.divergence.is_some(), "{:?}", d.lines());
        let q = w.run(&V2, 0, ok).unwrap().reason;
        assert!(
            matches!(q, Reason::Quarantined { .. }) && q.is_evidence(),
            "{q:?}"
        );

        let unprovable = Reason::Unprovable {
            why: "exit 1".into(),
        };
        assert!(!unprovable.is_evidence());
    }

    /// Trust v2's defaults: a result recorded once is replayable; the key's
    /// first two would-be hits are shadow audits (executed and compared);
    /// from then on a new manifest replays on its second identical run,
    /// with the decay auditing some. Fails on v1 at the first assertion.
    #[test]
    fn v2_replays_after_the_first_record_once_the_key_agreed_with_itself() {
        let mut w = World::new();
        let ok = b"ok  \tpkg\t0.4s\n";
        let v = w.run(&V2, 0, ok).unwrap();
        assert_eq!((v.reason.clone(), v.streak, v.needed), (Reason::Miss, 1, 1));
        assert!(v.replayable(), "recorded once: replayable");
        assert_eq!(v.shadow_left, 2);
        assert_eq!(
            v.lines(),
            [
                "trust: proof 1 of 1; replayable from the next matching run; its next 2 would-be hits are audited"
            ]
        );
        // Hit 1 and 2 of the key: shadow audits.
        let v = w.run(&V2, 0, ok).unwrap();
        assert_eq!(
            v.reason,
            Reason::Audit {
                nth: 1,
                shadow: true
            }
        );
        assert_eq!(v.headline(), "  (shadow audit: would-be hit 1 re-executed)");
        assert_eq!(
            v.lines(),
            [
                "trust: shadow audit passed, byte-identical to run 01RUN01; one more shadow audit before replays; audited 1 time, 0 divergences"
            ]
        );
        assert_eq!((v.streak, v.outcome()), (2, "audit"));
        let v = w.run(&V2, 0, b"ok  \tpkg\t0.9s\n").unwrap();
        assert_eq!(
            v.reason,
            Reason::Audit {
                nth: 2,
                shadow: true
            }
        );
        assert!(!v.byte_identical && v.divergence.is_none());
        assert_eq!(v.trust.volatile, 1, "identical up to a duration: volatile");
        assert_eq!(v.shadow_left, 0);
        assert!(
            v.lines()[0].contains("the shadow phase is over"),
            "{:?}",
            v.lines()
        );
        // Hit 3: the first blind replay.
        assert_eq!(w.run(&V2, 0, ok), Err(Decision::Replay));
        let audits = w.ledger.audits(Some(&w.key), 10).unwrap();
        assert_eq!(
            audits.iter().map(|a| a.phase.as_str()).collect::<Vec<_>>(),
            ["shadow", "shadow"]
        );
        // Another commit (manifest): recorded once, replays on its second
        // run unless the decay schedules an audit there (hit 4 is one).
        w.switch_manifest(b"module v2");
        let v = w.run(&V2, 0, ok).unwrap();
        assert_eq!((v.reason.clone(), v.streak), (Reason::Miss, 1));
        assert!(v.replayable() && v.shadow_left == 0);
        assert_eq!(
            v.lines(),
            ["trust: proof 1 of 1; replayable from the next matching run"]
        );
        let v = w.run(&V2, 0, ok).unwrap();
        assert_eq!(
            v.reason,
            Reason::Audit {
                nth: 4,
                shadow: false
            }
        );
        assert_eq!(v.headline(), "  (audit: would-be hit 4 re-executed)");
        assert_eq!(w.run(&V2, 0, ok), Err(Decision::Replay));
        // A third commit: its second run replays (hit 6 is not audited).
        w.switch_manifest(b"module v3");
        assert!(w.run(&V2, 0, ok).unwrap().replayable());
        assert_eq!(w.run(&V2, 0, ok), Err(Decision::Replay));
        let t = w.ledger.trust(&w.key).unwrap().unwrap();
        assert_eq!(
            (t.eligible, t.replays, t.audits, t.divergences),
            (6, 3, 3, 0)
        );
        assert_eq!(determinism(&t, 0), Determinism::Normalized);
        let trust = Trust::new(&w.ledger, &w.cas, V2).unwrap();
        assert_eq!(
            trust.facts(&w.key, &w.mid).unwrap().receipt_line(),
            "trust: proven by 1 identical execution, audited 3 times, 0 divergences"
        );
        // Under v1 the same ledger would still be proving the new manifest.
        assert!(matches!(
            Trust::new(&w.ledger, &w.cas, REPLAY)
                .unwrap()
                .decide(
                    &w.key,
                    w.ledger
                        .latest_result(&w.key, &w.mid)
                        .unwrap()
                        .map(|r| (w.manifest.clone(), r))
                        .as_ref()
                )
                .unwrap(),
            Decision::Execute(Reason::Proving {
                streak: 1,
                needed: 2
            })
        ));
    }

    /// A step printing random bytes is never served under v2 either: its
    /// first would-be hit is a shadow audit, which diverges and quarantines
    /// the key; nothing was replayed, so nothing is suspect.
    #[test]
    fn v2_a_nondeterministic_step_never_serves_a_replay() {
        let mut w = World::new();
        for i in 0..6u8 {
            let v = w.run(&V2, 0, &[i, i.wrapping_mul(7), 0xff]).unwrap();
            assert!(!v.replayable() || i == 0, "run {i}: {v:?}");
            assert_eq!(v.suspects, 0);
            if i == 1 {
                assert_eq!(
                    v.reason,
                    Reason::Audit {
                        nth: 1,
                        shadow: true
                    }
                );
            }
            if i > 0 {
                assert!(v.divergence.is_some(), "run {i}");
                assert_eq!(v.trust.state, TrustState::Quarantined);
                assert_eq!(
                    v.trust.shadow_from, 1,
                    "the shadow restarts at the quarantine"
                );
            }
        }
        assert_eq!(w.ledger.trust(&w.key).unwrap().unwrap().replays, 0);
        assert!(w.ledger.suspect_runs(None).unwrap().is_empty());
    }

    /// A divergence found by a scheduled audit marks the replays the key
    /// served before it suspect (they may have been wrong), restarts and
    /// lengthens the shadow, and raises the floor and the bar.
    #[test]
    fn v2_a_quarantine_marks_earlier_replays_suspect_and_restarts_the_shadow() {
        let p = Policy {
            mode: Mode::Replay,
            proofs: 1,
            shadow: 1,
            audit: 0.5,
            replay_failures: false,
            defer: false,
        };
        let s = Schedule::of(&p, 0);
        assert_eq!((0..4).filter(|k| s.due(*k)).collect::<Vec<_>>(), [0, 2]);
        let mut w = World::new();
        let ok = b"ok\n";
        w.run(&p, 0, ok).unwrap(); // recorded
        let v = w.run(&p, 0, ok).unwrap(); // hit 1: shadow
        assert!(matches!(v.reason, Reason::Audit { shadow: true, .. }));
        assert_eq!(w.run(&p, 0, ok), Err(Decision::Replay)); // hit 2: served
        // Hit 3: audited at the floor; the step now prints something else.
        let v = w.run(&p, 0, b"other\n").unwrap();
        assert!(matches!(
            v.reason,
            Reason::Audit {
                nth: 3,
                shadow: false
            }
        ));
        assert!(v.divergence.is_some());
        assert_eq!(v.suspects, 1, "the one replay served is suspect");
        assert!(
            v.lines()
                .iter()
                .any(|l| l.contains("1 earlier replay of this key may have served a wrong result")),
            "{:?}",
            v.lines()
        );
        let suspects = w.ledger.suspect_runs(Some(&w.key)).unwrap();
        assert_eq!(suspects.len(), 1);
        assert_eq!(suspects[0].outcome.as_deref(), Some("replayed"));
        let why = suspects[0].suspect.as_deref().unwrap();
        assert!(
            why.contains("key quarantined after a divergence: stdout differs from line 1")
                && why.ends_with("(audit 2)"),
            "{why}"
        );
        assert_eq!((v.trust.eligible, v.trust.shadow_from), (3, 3));
        assert_eq!(v.needed, 2, "the bar rose");
        let stricter = Schedule::of(&p, 1);
        assert_eq!((stricter.shadow, stricter.floor), (2, 1.0));
        assert_eq!(v.shadow_left, 2);
        // Re-proof: two identical executions of the new result.
        let v = w.run(&p, 0, b"other\n").unwrap();
        assert!(matches!(v.reason, Reason::Quarantined { .. }));
        assert_eq!((v.streak, v.trust.state), (2, TrustState::Proven));
        assert!(v.suspects == 0, "nothing new to mark");
        // Then the longer shadow (2) and the raised floor (every hit).
        for nth in 4..=6 {
            let v = w.run(&p, 0, b"other\n").unwrap();
            assert_eq!(
                v.reason,
                Reason::Audit {
                    nth,
                    shadow: nth <= 5
                },
                "hit {nth}"
            );
        }
        // The earlier suspect stays marked after the re-proof.
        assert_eq!(w.ledger.suspect_runs(None).unwrap().len(), 1);
        assert_eq!(
            determinism(&w.ledger.trust(&w.key).unwrap().unwrap(), 1),
            Determinism::Divergent
        );
    }

    /// A divergence without a result (the hooks: an offline run that
    /// misbehaved) marks suspects and restarts the shadow the same way.
    #[test]
    fn v2_a_divergence_without_a_result_marks_suspects() {
        let mut w = World::new();
        let ok = b"ok\n";
        let p = Policy { shadow: 1, ..V2 };
        w.run(&p, 0, ok).unwrap();
        w.run(&p, 0, ok).unwrap(); // shadow audit
        assert_eq!(w.run(&p, 0, ok), Err(Decision::Replay));
        assert_eq!(w.run(&p, 0, ok), Err(Decision::Replay));
        let hit = w
            .ledger
            .latest_result(&w.key, &w.mid)
            .unwrap()
            .map(|r| (w.manifest.clone(), r))
            .unwrap();
        let trust = Trust::new(&w.ledger, &w.cas, p).unwrap();
        let v = trust
            .record_divergence(
                &Step {
                    key: w.key,
                    cmd: "go test ./...",
                    ignore: &[],
                },
                "01FRESH",
                &hit,
                &Reason::Audit {
                    nth: 4,
                    shadow: false,
                },
                "tried to reach the network offline",
            )
            .unwrap();
        assert_eq!(v.suspects, 2);
        assert_eq!(v.trust.shadow_from, v.trust.eligible);
        assert_eq!(w.ledger.suspect_runs(Some(&w.key)).unwrap().len(), 2);
    }

    /// Deferred audits (docs/trust-v2.md, "Deferred audits"): the shadow
    /// stays inline; a decay-phase audit serves the replay and leaves a
    /// pending audit, which its later execution settles without counting
    /// as another would-be hit; at most `MAX_DEFERRED_PER_KEY` are owed
    /// before the next due audit runs inline again.
    #[test]
    fn deferred_audits_serve_the_replay_first_and_settle_later() {
        let ok = b"ok  \tpkg\t0.4s\n";
        let mut w = World::new();
        assert_eq!(w.run(&V2D, 0, ok).unwrap().reason, Reason::Miss);
        for nth in 1..=2 {
            let v = w.run(&V2D, 0, ok).unwrap();
            assert_eq!(
                v.reason,
                Reason::Audit { nth, shadow: true },
                "shadow audits are inline"
            );
        }
        assert_eq!(
            w.run(&V2D, 0, ok),
            Err(Decision::Replay),
            "hit 3 replays blind"
        );
        assert_eq!(
            w.run(&V2D, 0, ok),
            Err(Decision::Defer { nth: 4 }),
            "hit 4 is due: deferred"
        );
        let t = w.ledger.trust(&w.key).unwrap().unwrap();
        assert_eq!(
            (t.eligible, t.replays, t.audits),
            (4, 2, 2),
            "the replay counted, the audit not yet"
        );
        assert_eq!(
            w.ledger.backlog(&w.key, &stale_cutoff()).unwrap(),
            Backlog {
                pending: 1,
                owed: 0
            }
        );
        let pending = &w
            .ledger
            .pending_audits(Some(&w.key), quod_ledger::PendingState::Pending)
            .unwrap()[0];
        assert_eq!(
            (
                pending.nth,
                pending.replay_run.as_str(),
                pending.hit_run.as_str()
            ),
            (4, "01RUN05", "01RUN03"),
            "the hit is the manifest's newest result"
        );
        // Hits 5 and 6 replay meanwhile (the schedule has no audit there).
        assert_eq!(w.run(&V2D, 0, ok), Err(Decision::Replay));
        assert_eq!(w.run(&V2D, 0, ok), Err(Decision::Replay));
        // The worker executes it: compared with the served result, an audit
        // row in phase `deferred`, settled as passed; not a would-be hit.
        let v = w.audit(&V2D, 0, ok);
        assert_eq!(
            v.reason,
            Reason::DeferredAudit {
                nth: 4,
                pending: pending.id
            }
        );
        assert!(v.reason.is_evidence() && v.divergence.is_none() && v.byte_identical);
        assert_eq!(v.outcome(), "audit");
        assert_eq!(
            v.headline(),
            format!(
                "  (deferred audit {}: would-be hit 4 re-executed after its replay was served)",
                pending.id
            )
        );
        assert_eq!(
            v.lines(),
            [
                "trust: deferred audit passed, byte-identical to run 01RUN03; the replay it checked was right; audited 3 times, 0 divergences"
            ]
        );
        let t = w.ledger.trust(&w.key).unwrap().unwrap();
        assert_eq!(
            (t.eligible, t.replays, t.audits, t.state),
            (6, 4, 3, TrustState::Proven)
        );
        let settled = w.ledger.pending_audit(pending.id).unwrap().unwrap();
        assert_eq!(settled.state, quod_ledger::PendingState::Passed);
        assert!(settled.audit_id.is_some() && settled.detail.as_deref() == Some("byte-identical"));
        assert_eq!(
            w.ledger.backlog(&w.key, &stale_cutoff()).unwrap(),
            Backlog::default()
        );
        let phases: Vec<String> = w
            .ledger
            .audits(Some(&w.key), 10)
            .unwrap()
            .iter()
            .map(|a| a.phase.clone())
            .collect();
        assert_eq!(phases, ["deferred", "shadow", "shadow"]);
        // Hit 7 is the next on the schedule: deferred again.
        assert_eq!(w.run(&V2D, 0, ok), Err(Decision::Defer { nth: 7 }));
        // With deferral off the same hit executes inline.
        let mut inline = World::new();
        inline.run(&V2, 0, ok).unwrap();
        inline.run(&V2, 0, ok).unwrap();
        inline.run(&V2, 0, ok).unwrap();
        assert_eq!(inline.run(&V2, 0, ok), Err(Decision::Replay));
        assert_eq!(
            inline.run(&V2, 0, ok).unwrap().reason,
            Reason::Audit {
                nth: 4,
                shadow: false
            }
        );

        // The cap: every hit due (floor 1) defers two, then the third runs
        // inline until one settles.
        let all = Policy { audit: 1.0, ..V2D };
        let mut c = World::new();
        c.run(&all, 0, ok).unwrap();
        c.run(&all, 0, ok).unwrap();
        c.run(&all, 0, ok).unwrap();
        assert_eq!(c.run(&all, 0, ok), Err(Decision::Defer { nth: 3 }));
        assert_eq!(c.run(&all, 0, ok), Err(Decision::Defer { nth: 4 }));
        assert_eq!(
            c.run(&all, 0, ok).unwrap().reason,
            Reason::Audit {
                nth: 5,
                shadow: false
            }
        );
        c.audit(&all, 0, ok);
        assert_eq!(c.run(&all, 0, ok), Err(Decision::Defer { nth: 6 }));
        // Audit 0: nothing is deferred, nothing is audited.
        let none = Policy { audit: 0.0, ..V2D };
        assert_eq!(c.run(&none, 0, ok), Err(Decision::Replay));
    }

    /// A deferred audit that differs quarantines the key and marks every
    /// replay served since the last quarantine suspect, the one it checked
    /// and those served while it waited.
    #[test]
    fn a_deferred_audit_that_diverges_quarantines_and_marks_the_replays_suspect() {
        let ok = b"ok  \tpkg\t0.4s\n";
        let mut w = World::new();
        for _ in 0..3 {
            w.run(&V2D, 0, ok).unwrap();
        }
        assert_eq!(w.run(&V2D, 0, ok), Err(Decision::Replay));
        assert_eq!(w.run(&V2D, 0, ok), Err(Decision::Defer { nth: 4 }));
        assert_eq!(w.run(&V2D, 0, ok), Err(Decision::Replay));
        let v = w.audit(&V2D, 0, b"FAIL\tpkg\t0.4s\n");
        assert!(v.divergence.is_some(), "{v:?}");
        assert_eq!(v.outcome(), "diverged");
        assert_eq!(v.trust.state, TrustState::Quarantined);
        assert_eq!(
            v.suspects, 3,
            "hits 3, 4 (the one checked) and 5 were replays"
        );
        assert!(v.lines().iter().any(|l| l.contains("3 earlier replays of this key may have served a wrong result")), "{:?}", v.lines());
        let pending = &w
            .ledger
            .pending_audits(Some(&w.key), quod_ledger::PendingState::Diverged)
            .unwrap()[0];
        assert_eq!(pending.nth, 4);
        assert!(
            pending
                .detail
                .as_deref()
                .unwrap()
                .contains("stdout differs")
        );
        let suspects = w.ledger.suspect_runs(Some(&w.key)).unwrap();
        assert!(
            suspects.iter().any(|r| r.id == pending.replay_run),
            "the checked replay is suspect"
        );
        assert_eq!(suspects.len(), 3);
        // Quarantined: the next run executes to re-prove.
        assert!(matches!(
            w.run(&V2D, 0, ok).unwrap().reason,
            Reason::Quarantined { .. }
        ));
    }

    /// A lost deferred audit (never executed) is not passed: the key's next
    /// would-be hit is audited inline, schedule or not, and that repays it.
    #[test]
    fn a_lost_deferred_audit_is_repaid_by_the_next_hit_inline() {
        let ok = b"ok  \tpkg\t0.4s\n";
        let mut w = World::new();
        for _ in 0..3 {
            w.run(&V2D, 0, ok).unwrap();
        }
        assert_eq!(w.run(&V2D, 0, ok), Err(Decision::Replay));
        assert_eq!(w.run(&V2D, 0, ok), Err(Decision::Defer { nth: 4 }));
        let id = w
            .ledger
            .pending_audits(Some(&w.key), quod_ledger::PendingState::Pending)
            .unwrap()[0]
            .id;
        w.ledger.lose_audit(id, "expired").unwrap();
        assert_eq!(
            w.ledger.backlog(&w.key, &stale_cutoff()).unwrap(),
            Backlog {
                pending: 0,
                owed: 1
            }
        );
        let t = w.ledger.trust(&w.key).unwrap().unwrap();
        assert_eq!(t.audits, 2, "a lost audit never counts as one");
        // Hit 5 is not on the schedule, yet it is audited, inline.
        let v = w.run(&V2D, 0, ok).unwrap();
        assert_eq!(
            v.reason,
            Reason::Audit {
                nth: 5,
                shadow: false
            }
        );
        assert!(v.repaid);
        assert_eq!(
            v.lines(),
            [
                "trust: audit passed, byte-identical to run 01RUN03 (making up for a lost deferred audit); audited 3 times, 0 divergences"
            ]
        );
        assert_eq!(
            w.ledger.backlog(&w.key, &stale_cutoff()).unwrap(),
            Backlog::default()
        );
        assert!(w.ledger.pending_audit(id).unwrap().unwrap().repaid);
        // The schedule goes on from where it was: hit 6 replays, hit 7 is due.
        assert_eq!(w.run(&V2D, 0, ok), Err(Decision::Replay));
        assert_eq!(w.run(&V2D, 0, ok), Err(Decision::Defer { nth: 7 }));
    }

    /// The reviewer's case: two audits of a key pending; a hidden input
    /// changes; the first diverges and quarantines the key; the second
    /// would compare the new behaviour against the stale served result and
    /// quarantine again. It is dropped at the first quarantine instead, as
    /// repaid, so the re-proof's replays are never marked by it.
    #[test]
    fn a_quarantine_drops_the_keys_other_pending_audits() {
        let ok = b"ok  \tpkg\t0.4s\n";
        let all = Policy { audit: 1.0, ..V2D };
        let mut w = World::new();
        for _ in 0..3 {
            w.run(&all, 0, ok).unwrap();
        }
        assert_eq!(w.run(&all, 0, ok), Err(Decision::Defer { nth: 3 }));
        assert_eq!(w.run(&all, 0, ok), Err(Decision::Defer { nth: 4 }));
        let pending = w
            .ledger
            .pending_audits(Some(&w.key), quod_ledger::PendingState::Pending)
            .unwrap();
        assert_eq!(pending.len(), 2);
        let changed = b"ok  \tpkg\t0.4s\nnew\n";
        let v = w.audit(&all, 0, changed);
        assert!(v.divergence.is_some() && v.trust.state == TrustState::Quarantined);
        assert_eq!((v.suspects, v.superseded), (2, 1));
        assert!(
            v.lines()
                .iter()
                .any(|l| l.contains("1 deferred audit of this key still outstanding dropped")),
            "{:?}",
            v.lines()
        );
        let second = w.ledger.pending_audit(pending[1].id).unwrap().unwrap();
        assert!(second.state == quod_ledger::PendingState::Lost && second.repaid);
        assert!(
            second
                .detail
                .unwrap()
                .starts_with("superseded by the quarantine")
        );
        assert_eq!(
            w.ledger.backlog(&w.key, &stale_cutoff()).unwrap(),
            Backlog::default()
        );
        assert!(
            w.ledger.claim_audit("w").unwrap().is_none(),
            "nothing left to run"
        );
        // The key re-proves with the new result (bar 2) and replays it; the
        // quarantine was counted once.
        assert!(matches!(
            w.run(&all, 0, changed).unwrap().reason,
            Reason::Quarantined { .. }
        ));
        let v = w.run(&all, 0, changed).unwrap();
        assert!(v.replayable(), "{v:?}");
        assert_eq!(v.trust.divergences, 1);
        // An inline divergence drops them too.
        let mut i = World::new();
        for _ in 0..3 {
            i.run(&all, 0, ok).unwrap();
        }
        assert_eq!(i.run(&all, 0, ok), Err(Decision::Defer { nth: 3 }));
        let v = i
            .run(
                &Policy {
                    defer: false,
                    ..all
                },
                0,
                changed,
            )
            .unwrap();
        assert!(v.divergence.is_some());
        assert_eq!(v.superseded, 1);
        assert_eq!(
            i.ledger.backlog(&i.key, &stale_cutoff()).unwrap(),
            Backlog::default()
        );
    }

    #[test]
    fn determinism_classes() {
        let mut t = TrustRow::new(Hash::of(b"k"), None);
        assert_eq!(determinism(&t, 0), Determinism::Unknown);
        t.audits = 3;
        assert_eq!(determinism(&t, 0), Determinism::Exact);
        t.volatile = 1;
        assert_eq!(determinism(&t, 0), Determinism::Normalized);
        assert_eq!(determinism(&t, 1), Determinism::Divergent);
        assert_eq!(Determinism::Normalized.as_str(), "normalized");
        assert!(Determinism::Divergent.describe().contains("longer shadow"));
    }
}

#[cfg(test)]
mod transition_tests {
    use super::*;

    fn input<'a>(
        reason: &'a Reason,
        prior: Option<ProofRow>,
        compared: Compared,
        fp: Hash,
    ) -> ExecutionInput<'a> {
        ExecutionInput {
            key: Hash::of(b"k"),
            cmd: "go test ./...",
            run_id: "02RUN",
            manifest: Hash::of(b"m"),
            fingerprint: fp,
            unprovable: None,
            reason,
            trust: None,
            prior,
            compared,
            recent_divergences: 0,
            hit_duration_ms: Some(1000),
            now: "2026-11-01T00:00:00.000Z".into(),
        }
    }

    fn prior(streak: u32, fp: Hash) -> ProofRow {
        ProofRow {
            step_key: Hash::of(b"k"),
            manifest_id: Hash::of(b"m"),
            streak,
            fingerprint: fp,
            last_run: "01RUN".into(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn a_first_execution_starts_a_streak_of_one() {
        let t = execution_transition(
            &Policy::default(),
            input(&Reason::Miss, None, Compared::default(), Hash::of(b"fp")),
        );
        assert_eq!(t.proof.streak, 1);
        // Trust v2: one record proves the manifest (proofs 1); the shadow
        // audits stand between it and the first blind replay.
        assert_eq!(t.trust.state, TrustState::Proven);
        assert!(t.verdict.replayable());
        assert_eq!(t.verdict.shadow_left, u64::from(DEFAULT_SHADOW));
        assert!(t.audit.is_none() && !t.reset_proofs && !t.mark_suspects);
        assert_eq!(t.verdict.outcome(), "miss");
    }

    #[test]
    fn the_same_result_again_proves_the_key() {
        let fp = Hash::of(b"fp");
        let c = Compared {
            byte_identical: true,
            previous: Some(Some(fp)),
            compared_with: Some("01RUN".into()),
            ..Compared::default()
        };
        let reason = Reason::Proving {
            streak: 1,
            needed: 2,
        };
        let t = execution_transition(
            &Policy::default(),
            input(&reason, Some(prior(1, fp)), c, fp),
        );
        assert_eq!(t.proof.streak, 2);
        assert_eq!(t.trust.state, TrustState::Proven);
        assert_eq!(t.trust.audits, 1);
        let a = t.audit.expect("a comparison is an audit");
        assert!(!a.diverged);
        assert_eq!(a.detail.as_deref(), Some("byte-identical"));
        assert!(t.verdict.replayable());
    }

    #[test]
    fn a_different_result_quarantines_and_resets_every_streak() {
        let c = Compared {
            byte_identical: true,
            previous: Some(Some(Hash::of(b"old"))),
            compared_with: Some("01RUN".into()),
            ..Compared::default()
        };
        let t = execution_transition(
            &Policy::default(),
            input(
                &Reason::WouldReplay,
                Some(prior(5, Hash::of(b"old"))),
                c,
                Hash::of(b"new"),
            ),
        );
        assert!(t.reset_proofs && t.mark_suspects);
        assert_eq!(t.proof.streak, 1);
        assert_eq!(t.trust.state, TrustState::Quarantined);
        assert_eq!(t.trust.divergences, 1);
        assert_eq!(t.trust.eligible, 1);
        // The shadow phase starts over, twice as long after a divergence.
        assert_eq!(t.trust.shadow_from, t.trust.eligible);
        assert_eq!(t.verdict.shadow_left, 2 * u64::from(DEFAULT_SHADOW));
        assert_eq!(t.verdict.needed, DEFAULT_PROOFS + 1);
        assert!(t.audit.unwrap().diverged);
        assert_eq!(t.verdict.outcome(), "diverged");
    }

    #[test]
    fn an_unprovable_result_never_counts() {
        let fp = Hash::of(b"fp");
        let mut i = input(
            &Reason::Miss,
            Some(prior(1, fp)),
            Compared {
                previous: Some(Some(fp)),
                compared_with: Some("01RUN".into()),
                byte_identical: true,
                ..Compared::default()
            },
            fp,
        );
        i.unprovable = Some("exit code 1".into());
        let t = execution_transition(&Policy::default(), i);
        assert_eq!(t.proof.streak, 0);
        assert_ne!(t.trust.state, TrustState::Proven);
    }

    #[test]
    fn a_gone_result_falls_back_to_the_stored_fingerprint() {
        let fp = Hash::of(b"fp");
        let c = Compared {
            byte_identical: true,
            previous: Some(None),
            ..Compared::default()
        };
        let t = execution_transition(
            &Policy::default(),
            input(&Reason::Miss, Some(prior(1, fp)), c, fp),
        );
        assert_eq!(t.proof.streak, 2);
        assert_eq!(t.verdict.compared_with.as_deref(), Some("01RUN"));
        assert!(!t.verdict.byte_identical);
        // A mismatch restarts the streak and quarantines nothing.
        let c = Compared {
            byte_identical: true,
            previous: Some(None),
            ..Compared::default()
        };
        let t = execution_transition(
            &Policy::default(),
            input(&Reason::Miss, Some(prior(1, fp)), c, Hash::of(b"other")),
        );
        assert_eq!(t.proof.streak, 1);
        assert!(!t.reset_proofs && t.audit.is_none());
    }

    #[test]
    fn a_divergence_without_a_result_quarantines() {
        let reason = Reason::Audit {
            nth: 3,
            shadow: false,
        };
        let t = divergence_transition(
            &Policy::default(),
            DivergenceInput {
                key: Hash::of(b"k"),
                cmd: "go test ./...",
                run_id: "02RUN",
                manifest: Hash::of(b"m"),
                hit_run: "01RUN",
                reason: &reason,
                why: "reached for the network",
                trust: None,
                recent_divergences: 0,
                now: "2026-11-01T00:00:00.000Z".into(),
            },
        );
        assert_eq!(t.trust.state, TrustState::Quarantined);
        assert_eq!(
            (t.trust.eligible, t.trust.audits, t.trust.divergences),
            (1, 1, 1)
        );
        assert_eq!(t.verdict.needed, DEFAULT_PROOFS + 1);
        assert_eq!(t.trust.shadow_from, 1);
        assert_eq!(t.verdict.shadow_left, 2 * u64::from(DEFAULT_SHADOW));
        assert!(t.audit.diverged);
        assert_eq!(t.audit.phase, "audit");
    }

    #[test]
    fn a_comparison_identical_only_after_normalizing_is_volatile() {
        let fp = Hash::of(b"fp");
        let reason = Reason::Audit {
            nth: 1,
            shadow: true,
        };
        let c = Compared {
            byte_identical: false,
            previous: Some(Some(fp)),
            compared_with: Some("01RUN".into()),
            ..Compared::default()
        };
        let t = execution_transition(
            &Policy::default(),
            input(&reason, Some(prior(1, fp)), c, fp),
        );
        assert_eq!(t.trust.volatile, 1);
        let a = t.audit.unwrap();
        assert_eq!(a.phase, "shadow");
        assert_eq!(
            a.detail.as_deref(),
            Some("identical up to normalized volatile fields")
        );
        // Only a stored fingerprint compared: no volatile sighting.
        let c = Compared {
            byte_identical: true,
            previous: Some(None),
            ..Compared::default()
        };
        let t = execution_transition(
            &Policy::default(),
            input(&reason, Some(prior(1, fp)), c, fp),
        );
        assert_eq!(t.trust.volatile, 0);
        assert_eq!(
            t.audit.unwrap().detail.as_deref(),
            Some("identical by stored fingerprint (previous result purged)")
        );
    }

    #[test]
    fn wire_types_round_trip() {
        let r = Reason::Quarantined {
            since: "x".into(),
            why: "y".into(),
            streak: 1,
            needed: 3,
        };
        let j = serde_json::to_string(&r).unwrap();
        assert_eq!(serde_json::from_str::<Reason>(&j).unwrap(), r);
        let a = Reason::Audit {
            nth: 4,
            shadow: true,
        };
        let j = serde_json::to_string(&a).unwrap();
        assert_eq!(serde_json::from_str::<Reason>(&j).unwrap(), a);
        let c = Claimed::Execute { reason: r };
        let j = serde_json::to_string(&c).unwrap();
        assert_eq!(serde_json::from_str::<Claimed>(&j).unwrap(), c);
    }
}
