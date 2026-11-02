//! Test-level replay in `quod run` (docs/test-level.md): plan which tests
//! of a pytest step replay, run the step with Quod's plugin, attribute its
//! reads to its tests, and record each test under its own key, where the
//! trust layer (proofs, the shadow and decay audits, quarantine, suspect
//! marking) applies to it unchanged.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::Context;
use quod_cas::{Cas, Hash, Kind, Tree};
use quod_ledger::{Ledger, Observed, RunFacts, now_rfc3339};
use quod_manifest::{Access, Manifest, StepResult};
use quod_tests::{Attribution, Record};
use quod_trust::{Decision, Reason, Replayed, Step, Trust};

/// Whether a step runs at test level: a Python step that runs pytest,
/// unless `QUOD_TEST_LEVEL=0` turns it off.
pub(crate) fn applies(toolchain: quod_key::Toolchain, script: &str) -> bool {
    toolchain == quod_key::Toolchain::Python
        && quod_tests::runs_pytest(script)
        && !std::env::var(quod_tests::KEY_ENV).is_ok_and(|v| v == "0")
}

/// One known test of the step, as planned.
pub(crate) struct Planned {
    pub(crate) key: Hash,
    pub(crate) hit: Option<(Manifest, StepResult)>,
    pub(crate) decision: Decision,
}

/// What a test-level run was told, and where its files are.
pub(crate) struct Plan {
    pub(crate) token: String,
    /// The host directory bound at `quod_tests::SANDBOX_DIR`.
    pub(crate) dir: PathBuf,
    /// Why every test executes this time, if it does.
    pub(crate) full: Option<String>,
    pub(crate) tests: BTreeMap<String, Planned>,
    pub(crate) session_hit: Option<(Manifest, StepResult)>,
    /// The tests planned to replay, with their records.
    pub(crate) replay: BTreeMap<String, Record>,
    /// Tests that failed in an attempt that replayed others, with what they
    /// recorded there: the full run that follows compares each with it.
    pub(crate) attempt: BTreeMap<String, (Manifest, StepResult)>,
    /// The step's coupled test modules (`TestStepRow::coupled`).
    pub(crate) coupled: Vec<String>,
}

/// A test's module: its node id up to the first `::`.
pub(crate) fn module_of(nodeid: &str) -> &str {
    nodeid.split("::").next().unwrap_or(nodeid)
}

/// What the sandbox needs from a plan (`run::execute`).
pub(crate) struct Exec<'a> {
    pub(crate) token: &'a str,
    pub(crate) dir: &'a Path,
}

impl Plan {
    pub(crate) fn exec(&self) -> Exec<'_> {
        Exec {
            token: &self.token,
            dir: &self.dir,
        }
    }

    /// Every test executes from now on (a rerun to confirm a failure, or
    /// a run whose attribution failed): the plan is rewritten empty and the
    /// previous attempt's outputs are cleared.
    pub(crate) fn go_full(&mut self, why: &str) -> anyhow::Result<()> {
        self.full = Some(why.to_string());
        self.replay.clear();
        self.write()
    }

    /// Writes the plugin, the plan and an empty output directory.
    pub(crate) fn write(&self) -> anyhow::Result<()> {
        let _ = std::fs::remove_dir_all(&self.dir);
        std::fs::create_dir_all(self.dir.join("py"))?;
        std::fs::create_dir_all(self.dir.join("out"))?;
        std::fs::write(
            self.dir
                .join("py")
                .join(format!("{}.py", quod_tests::PLUGIN_MODULE)),
            quod_tests::PLUGIN_SOURCE,
        )?;
        let plan = quod_tests::Plan {
            token: self.token.clone(),
            replay: self.replay.clone(),
        };
        std::fs::write(self.dir.join("plan.json"), serde_json::to_vec(&plan)?)?;
        // The sandbox runs as another uid: it must write its outputs here.
        use std::os::unix::fs::PermissionsExt;
        for d in [self.dir.clone(), self.dir.join("out")] {
            std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o777))?;
        }
        Ok(())
    }

    /// The plugin's outputs and the trace, attributed. `Err` says why the
    /// run cannot be recorded at test level.
    pub(crate) fn attribute(
        &self,
        trace: Option<&quod_fs::Trace>,
        extra: &[Access],
    ) -> Result<Attribution, String> {
        let trace = trace.ok_or("the observer kept no trace")?;
        let outs = quod_tests::read_outputs(&self.dir).map_err(|e| e.to_string())?;
        let mut a = quod_tests::attribute(trace, &outs).map_err(|e| e.to_string())?;
        // Sealed dependency trees and their carve-outs, recorded whole
        // rather than observed: every test's.
        a.session.extend(extra.iter().cloned());
        let planned: BTreeSet<&String> = self.replay.keys().collect();
        if let Some(t) = a.replayed.iter().find(|t| !planned.contains(t)) {
            return Err(format!(
                "the plugin replayed {t}, which the plan did not name"
            ));
        }
        Ok(a)
    }
}

/// The trust policy for tests: the step's, with every audit inline (a
/// test's audit costs that test's run, inside a run that happens anyway).
pub(crate) fn policy(step: &quod_trust::Policy) -> quod_trust::Policy {
    quod_trust::Policy {
        defer: false,
        ..*step
    }
}

/// Plans a test-level run of `step_key` on `tree`. `full`: why every test
/// must execute this time (the step-level decision was evidence about a
/// whole-step result, for instance), if it must.
pub(crate) fn plan(
    ledger: &Ledger,
    trust: &Trust,
    cas: &Cas,
    step_key: &Hash,
    tree: &Tree,
    run_dir: &Path,
    full: Option<String>,
) -> anyhow::Result<Plan> {
    let state = ledger.test_step(step_key)?;
    let session_hit = ledger
        .find_match(&quod_tests::session_key(step_key), tree)?
        .hit;
    let full = full
        .or_else(|| state.disabled.clone())
        .or_else(|| state.full_due.clone())
        .or_else(|| {
            (state.runs > 0 && state.since_full + 1 >= quod_tests::FULL_EVERY)
                .then(|| format!("every {}th run executes every test", quod_tests::FULL_EVERY))
        })
        .or_else(|| {
            session_hit.is_none().then(|| {
                "the session's inputs (conftest, config, imports, listings) changed or were never recorded"
                    .to_string()
            })
        });
    let mut tests = BTreeMap::new();
    let mut replay = BTreeMap::new();
    for (key, nodeid) in ledger.tests_for_step(step_key)? {
        let hit = ledger.find_match(&key, tree)?.hit;
        let decision = match trust.decide(&key, hit.as_ref())? {
            // Tests never defer (`policy`); inline if one ever does.
            Decision::Defer { nth } => Decision::Execute(Reason::Audit { nth, shadow: false }),
            d => d,
        };
        if full.is_none()
            && decision == Decision::Replay
            && let Some((_, r)) = &hit
            && let Some(bytes) = cas.get(&r.stdout)?
            && let Ok(rec) = Record::from_blob(&bytes)
        {
            replay.insert(nodeid.clone(), rec);
        }
        tests.insert(nodeid, Planned { key, hit, decision });
    }
    // A coupled module (a test there once failed beside replayed tests and
    // passed in the full run) runs whole or not at all.
    for m in &state.coupled {
        let runs = tests
            .keys()
            .any(|t| module_of(t) == m && !replay.contains_key(t));
        if runs {
            replay.retain(|t, _| module_of(t) != m);
        }
    }
    let token = ulid::Ulid::generate().to_string().to_lowercase();
    let plan = Plan {
        token,
        dir: run_dir.join("tl"),
        full,
        tests,
        session_hit,
        replay,
        attempt: BTreeMap::new(),
        coupled: state.coupled.clone(),
    };
    plan.write()?;
    Ok(plan)
}

/// The run row facts every test row shares.
pub(crate) struct RowInfo<'a> {
    pub(crate) run_id: &'a str,
    pub(crate) commit: Option<&'a str>,
    pub(crate) host: Option<&'a str>,
    pub(crate) repo: &'a str,
    pub(crate) repo_path: &'a str,
}

/// What `record` did, for the banner and the event.
#[derive(Debug, Default)]
pub(crate) struct Summary {
    pub(crate) collected: usize,
    pub(crate) replayed: usize,
    pub(crate) executed: usize,
    pub(crate) diverged: Vec<String>,
    pub(crate) late: Vec<String>,
    pub(crate) saved_ms: u64,
    /// An attempt's failed tests (`record_attempt`).
    pub(crate) failed: BTreeMap<String, (Manifest, StepResult)>,
    /// Modules newly coupled by this run.
    pub(crate) coupled: Vec<String>,
}

/// A test's run row: mode `test`, its id `<step run>.t<i>` (executed) or
/// `.r<i>` (replayed), so `quod status` and the dashboard can group them.
fn test_facts<'b>(row: &'b RowInfo<'_>, id: &'b str) -> RunFacts<'b> {
    RunFacts {
        id,
        commit_sha: row.commit,
        host: row.host,
        observed: Observed::default(),
        repo: Some(row.repo),
        repo_path: Some(row.repo_path),
        mode: Some("test"),
        ..RunFacts::default()
    }
}

fn accesses_of(m: &Manifest) -> impl Iterator<Item = Access> + '_ {
    m.entries.iter().map(|e| Access {
        path: e.p.clone(),
        kind: e.k,
    })
}

/// Records a test-level run: the session manifest, every executed test's
/// manifest and result through the trust layer, every replayed test's
/// replay, the step's test list and its test-level state. One transaction.
#[allow(clippy::too_many_arguments)] // one call site; the parts of a run, each named
pub(crate) fn record(
    ledger: &Ledger,
    trust: &Trust,
    cas: &Cas,
    plan: &Plan,
    att: &Attribution,
    step_key: &Hash,
    roots: &[String],
    tree: &Tree,
    row: &RowInfo,
) -> anyhow::Result<Summary> {
    record_with(
        ledger, trust, cas, plan, att, step_key, roots, tree, row, true,
    )
}

/// Records only the executed tests of an attempt that is run again in
/// full (`run`): each is evidence about its test, whatever replayed beside
/// it, so a test whose result depends on what else ran is compared with
/// its full-run result and diverges. Its replays were never the step's
/// result (the full run is), and its session is the full run's to record.
#[allow(clippy::too_many_arguments)] // one call site; the parts of a run, each named
pub(crate) fn record_attempt(
    ledger: &Ledger,
    trust: &Trust,
    cas: &Cas,
    plan: &Plan,
    att: &Attribution,
    step_key: &Hash,
    roots: &[String],
    tree: &Tree,
    row: &RowInfo,
) -> anyhow::Result<Summary> {
    record_with(
        ledger, trust, cas, plan, att, step_key, roots, tree, row, false,
    )
}

#[allow(clippy::too_many_arguments)]
fn record_with(
    ledger: &Ledger,
    trust: &Trust,
    cas: &Cas,
    plan: &Plan,
    att: &Attribution,
    step_key: &Hash,
    roots: &[String],
    tree: &Tree,
    row: &RowInfo,
    whole: bool,
) -> anyhow::Result<Summary> {
    let skey = quod_tests::session_key(step_key);
    let empty = cas.put(b"")?;
    ledger.atomically(|l| -> anyhow::Result<Summary> {
        let mut s = Summary {
            collected: att.collected.len(),
            ..Summary::default()
        };
        if whole {
            // The session: what this run attributed to it, and what the
            // matched session manifest held (it matched this tree, so the
            // union still describes it; a replayed test's listings and absent
            // names were recorded there by the run that recorded the test).
            let mut session: Vec<Access> = att.session.iter().cloned().collect();
            if let Some((m, _)) = &plan.session_hit {
                session.extend(accesses_of(m));
            }
            let sm = Manifest::from_log(skey, roots.to_vec(), tree, session)
                .context("the session manifest")?;
            let srun = format!("{}.s", row.run_id);
            let smid = l.put_manifest(&sm, &srun)?;
            l.put_result(&StepResult {
                v: quod_manifest::VERSION,
                step_key: skey,
                manifest_id: smid,
                run_id: srun.clone(),
                recorded_at: now_rfc3339(),
                exit_code: 0,
                stdout: empty,
                stderr: empty,
                outputs: vec![],
                root_outputs: vec![],
                duration_ms: 0,
                commands: vec![],
            })?;
            l.put_tests(
                step_key,
                row.run_id,
                &att.collected
                    .iter()
                    .map(|n| (quod_tests::test_key(step_key, n), n.clone()))
                    .collect::<Vec<_>>(),
            )?;
        }
        for (i, (nodeid, ex)) in att.executed.iter().enumerate() {
            let planned = plan.tests.get(nodeid);
            let key = planned
                .map(|p| p.key)
                .unwrap_or_else(|| quod_tests::test_key(step_key, nodeid));
            let attempt = if whole {
                plan.attempt.get(nodeid)
            } else {
                None
            };
            let hit = attempt.or_else(|| planned.and_then(|p| p.hit.as_ref()));
            let reason = match (attempt, planned.map(|p| &p.decision)) {
                // It failed beside replayed tests: this full run is
                // compared with that, and a different result is a
                // divergence (its result depended on what else ran).
                (Some(_), _) => Reason::Proving {
                    streak: 0,
                    needed: 1,
                },
                (None, None) => Reason::Miss,
                (None, Some(Decision::Execute(r))) => r.clone(),
                // Planned to replay but executed: a full run, or the
                // plugin could not replay it. Compared, as an audit is.
                (None, Some(_)) => Reason::WouldReplay,
            };
            let trun = format!("{}.t{i}", row.run_id);
            let manifest =
                Manifest::from_log(key, roots.to_vec(), tree, ex.accesses.iter().cloned())
                    .with_context(|| format!("the manifest of {nodeid}"))?;
            let mid = quod_manifest::manifest_id(&manifest)?;
            let result = StepResult {
                v: quod_manifest::VERSION,
                step_key: key,
                manifest_id: mid,
                run_id: trun.clone(),
                recorded_at: now_rfc3339(),
                exit_code: i32::from(ex.record.failed()),
                stdout: cas.put(&ex.record.to_blob())?,
                stderr: empty,
                outputs: vec![],
                root_outputs: vec![],
                duration_ms: ex.duration_ms,
                commands: vec![],
            };
            let step = Step {
                key,
                cmd: nodeid,
                ignore: &[],
            };
            let v = trust.record_execution(&step, &trun, &manifest, &result, hit, &reason)?;
            if v.divergence.is_some() {
                s.diverged.push(nodeid.clone());
                if attempt.is_some() {
                    s.coupled.push(module_of(nodeid).to_string());
                }
            }
            if !whole && result.exit_code != 0 {
                s.failed
                    .insert(nodeid.clone(), (manifest.clone(), result.clone()));
            }
            l.record_execution_facts(
                &RunFacts {
                    step_key: Some(&key),
                    outcome: Some(v.outcome()),
                    manifest_id: Some(&mid),
                    exit_code: Some(result.exit_code),
                    wall_ms: Some(ex.duration_ms),
                    ..test_facts(row, &trun)
                },
                &manifest,
                &result,
            )?;
            s.executed += 1;
        }
        let replayed = if whole {
            &att.replayed
        } else {
            &BTreeSet::new()
        };
        for (i, nodeid) in replayed.iter().enumerate() {
            let p = plan
                .tests
                .get(nodeid)
                .filter(|p| p.decision == Decision::Replay)
                .with_context(|| format!("{nodeid} replayed without a plan"))?;
            let hit = p.hit.as_ref().context("a replay needs a hit")?;
            let step = Step {
                key: p.key,
                cmd: nodeid,
                ignore: &[],
            };
            let trun = format!("{}.r{i}", row.run_id);
            let late = matches!(trust.record_replay(&step, hit)?, Replayed::Execute(_));
            l.record_run_facts(&RunFacts {
                step_key: Some(&p.key),
                outcome: Some("replayed"),
                manifest_id: Some(&hit.1.manifest_id),
                exit_code: Some(hit.1.exit_code),
                wall_ms: Some(0),
                saved_ms: Some(hit.1.duration_ms),
                ..test_facts(row, &trun)
            })?;
            if late {
                // Another run quarantined the test while this one served
                // it: name it to re-run.
                l.mark_suspect_replays(
                    &p.key,
                    &format!(
                        "{}: quarantined by another run while this run replayed it",
                        now_rfc3339()
                    ),
                )?;
                s.late.push(nodeid.clone());
            }
            s.saved_ms += hit.1.duration_ms;
            s.replayed += 1;
        }
        if whole {
            let mut st = l.test_step(step_key)?;
            st.runs += 1;
            st.since_full = if plan.full.is_some() {
                0
            } else {
                st.since_full + 1
            };
            st.full_due = s.diverged.first().map(|t| {
                format!(
                    "{t} diverged in run {}: the next run executes every test",
                    row.run_id
                )
            });
            for m in &s.coupled {
                if !st.coupled.contains(m) {
                    st.coupled.push(m.clone());
                }
            }
            l.put_test_step(&st)?;
        }
        Ok(s)
    })
}

/// The environment a test-level run adds (never keyed: the key carries
/// `quod_tests::KEY_ENV` instead): where the plugin, the plan and its
/// outputs are, the marker token and the workspace root, and the plugin
/// itself on PYTHONPATH and PYTEST_PLUGINS, after whatever the step sets.
pub(crate) fn sandbox_env(env: &mut Vec<(String, String)>, token: &str) {
    let dir = quod_tests::SANDBOX_DIR;
    let mut set = |k: &str, v: String, sep: Option<char>| match env.iter_mut().find(|(n, _)| n == k)
    {
        Some((_, old)) => match sep {
            Some(c) if !old.is_empty() => {
                *old = if k == "PYTHONPATH" {
                    format!("{v}{c}{old}")
                } else {
                    format!("{old}{c}{v}")
                }
            }
            _ => *old = v,
        },
        None => env.push((k.to_string(), v)),
    };
    set("QUOD_TL_DIR", dir.to_string(), None);
    set("QUOD_TL_TOKEN", token.to_string(), None);
    set("QUOD_TL_ROOT", quod_key::WORKSPACE.to_string(), None);
    set("PYTHONPATH", format!("{dir}/py"), Some(':'));
    set(
        "PYTEST_PLUGINS",
        quod_tests::PLUGIN_MODULE.to_string(),
        Some(','),
    );
}

/// The structured event of a test-level run (events.jsonl; the bench
/// reads it).
pub(crate) fn emit(
    run_id: &str,
    step_key: &Hash,
    plan: &Plan,
    att: &Attribution,
    s: &Summary,
    wall: std::time::Duration,
) {
    let files = att.session.iter().filter(|a| a.kind == Kind::File).count();
    tracing::info!(
        target: "quod",
        run_id,
        step_key = %step_key,
        collected = s.collected,
        replayed = s.replayed,
        executed = s.executed,
        diverged = s.diverged.len(),
        late = s.late.len(),
        planned = plan.replay.len(),
        known = plan.tests.len(),
        full = plan.full.as_deref(),
        session_inputs = att.session.len(),
        session_files = files,
        saved_ms = s.saved_ms,
        wall_ms = wall.as_millis() as u64,
        exitstatus = att.exitstatus,
        "tests"
    );
}

/// The receipt of a run that replayed tests.
pub(crate) fn banner(
    key: &Hash,
    plan: &Plan,
    att: &Attribution,
    s: &Summary,
    wall: std::time::Duration,
) -> String {
    let mut out = format!(
        "QUOD  EXECUTED AT TEST LEVEL  {} of {} tests replayed, {} ran in {} (replayed tests took {} when recorded)\n",
        s.replayed,
        s.collected,
        s.executed,
        quod_replay::fmt_duration(wall),
        quod_replay::fmt_duration(std::time::Duration::from_millis(s.saved_ms)),
    );
    out += &format!("  step key {key}; {}\n", session_line(att));
    if !s.diverged.is_empty() {
        out += &format!(
            "  {} test(s) diverged and are quarantined; the next run executes every test: {}\n",
            s.diverged.len(),
            s.diverged.join(", ")
        );
    }
    if !s.late.is_empty() {
        out += &format!(
            "  {} replayed test(s) were quarantined by another run meanwhile; re-run them: {}\n",
            s.late.len(),
            s.late.join(", ")
        );
    }
    let _ = plan;
    out
}

/// The line under a full test-level run's step receipt.
pub(crate) fn full_line(plan: &Plan, s: &Summary) -> String {
    let mut l = format!("test level: {} tests recorded", s.executed);
    if let Some(why) = &plan.full {
        l += &format!(", every test ran ({why})");
    } else if !plan.tests.is_empty() {
        l += ", none could replay";
    }
    if !s.diverged.is_empty() {
        l += &format!(
            "; {} diverged and are quarantined: {}",
            s.diverged.len(),
            s.diverged.join(", ")
        );
    }
    l
}

fn session_line(att: &Attribution) -> String {
    let files = att.session.iter().filter(|a| a.kind == Kind::File).count();
    format!(
        "every test depends on {} session inputs ({} files)",
        att.session.len(),
        files
    )
}
