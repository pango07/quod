//! The spike's acceptance criteria A1 to A7, in order, on one checkout, for
//! any toolchain profile.
//!
//! Runs on the tiny Go module by default (every PR), on the tiny Node and
//! Python fixtures when their toolchains are installed, or on a real repo
//! when STILLPASS_SPIKE_REPO is set (scripts/acceptance.sh, nightly) with
//! STILLPASS_TOOLCHAIN naming its profile and STILLPASS_STEP its test step.
//! The repo is copied first; the original is never touched.
//!
//! Everything language-specific comes from the profile: which file A4 edits
//! (a source file of the language the step read), how it is edited (the
//! language's line comment), whether warm state exists (W1-W4), and which
//! directories a step must never write under.
//!
//! A7b goes beyond the handoff: A7 mutates only paths the manifest
//! recorded, so it proves the matcher, not the observer. A7b mutates paths
//! the observer did NOT record, expects a hit (the model says they are not
//! inputs), and re-executes the step under a fresh key to check the result
//! really is unchanged. An unlogged read would show up as a difference.

mod common;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use common::*;
use sp_cas::{Cas, Entry, Hash, Kind, Tree, child_path};
use sp_key::Toolchain;
use sp_ledger::{Ledger, first_match};

#[test]
fn acceptance() {
    if !enabled() {
        return;
    }
    let spike = std::env::var("STILLPASS_SPIKE_REPO").ok();
    let tc: Toolchain = std::env::var("STILLPASS_TOOLCHAIN")
        .ok()
        .map(|s| s.parse().expect("STILLPASS_TOOLCHAIN"))
        .unwrap_or(Toolchain::Go);
    let env = match &spike {
        Some(src) => Env::from_copy(Path::new(src), Some(tc)),
        None => Env::fixture(tc),
    };
    let step = std::env::var("STILLPASS_STEP").unwrap_or_else(|_| tc.profile().test.to_string());
    let samples: usize = std::env::var("STILLPASS_A7B_SAMPLES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    run_acceptance(&env, tc, &step, samples, spike.is_some());
}

/// The Node fixture under the Node profile (npm, a sealed node_modules).
#[test]
fn acceptance_node_fixture() {
    if !enabled() || !toolchain_installed(Toolchain::Node) {
        return;
    }
    let env = Env::fixture(Toolchain::Node);
    run_acceptance(
        &env,
        Toolchain::Node,
        Toolchain::Node.profile().test,
        2,
        false,
    );
}

/// The Python fixture under the Python profile (a sealed venv, no pycs).
#[test]
fn acceptance_python_fixture() {
    if !enabled() || !toolchain_installed(Toolchain::Python) {
        return;
    }
    let env = Env::fixture(Toolchain::Python);
    run_acceptance(
        &env,
        Toolchain::Python,
        "python -m unittest discover -s tests -t . -v",
        2,
        false,
    );
}

fn run_acceptance(env: &Env, tc: Toolchain, step: &str, samples: usize, real_repo: bool) {
    let profile = tc.profile();
    let ledger = || Ledger::open(&env.state().join("ledger.sqlite")).unwrap();

    // Where `stillpass run` keeps this repo's `.stillpass` (its dependency
    // trees, crate::deps): a network step's banner names it, so a no-op one
    // asks. On a real repo the script's step A filled it (STILLPASS_DEPS_DIR
    // is shared); a fixture's tree is adopted from its checkout. A7 and A7b
    // graft its dependency trees into the trees they check, as `stillpass
    // run` does.
    let probe = env.run_step("true", &["--allow-net"]);
    assert_eq!(probe.code, 0, "deps probe: {}", probe.stderr);
    let deps: PathBuf = probe
        .stderr
        .lines()
        .find_map(|l| l.trim_start().strip_prefix("caches in "))
        .map(PathBuf::from)
        .expect("the network step's banner names its caches dir");
    if real_repo {
        assert!(
            !env.repo.join(".stillpass").exists(),
            "step A left a .stillpass in the checkout"
        );
        println!(
            "step A ok: dependency trees in {}, none in the checkout",
            deps.display()
        );
    }

    // A1: first run executes and records one manifest and result.
    let a1 = env.run_step(step, &[]);
    assert!(a1.executed() && a1.code == 0, "A1: {}", a1.stderr);
    let key = only_key(env);
    assert_eq!(
        ledger().manifest_count(&key).unwrap(),
        1,
        "A1: one manifest"
    );
    let inspect = env.stillpass(&["inspect", &key.to_string()]);
    assert!(String::from_utf8_lossy(&inspect.stdout).contains("1 manifest(s)"));
    // The profile's env keeps the tool's own caches off the workspace.
    let result = ledger()
        .results_for_run(&newest_run(env))
        .unwrap()
        .remove(0);
    let forbidden: Vec<&str> = result
        .outputs
        .iter()
        .filter(|o| profile.forbids_output(&o.p))
        .map(|o| o.p.as_str())
        .collect();
    assert!(
        forbidden.is_empty(),
        "A1: the step wrote under a directory the {tc} profile must keep it from writing: {:?}",
        &forbidden[..forbidden.len().min(5)]
    );
    println!(
        "A1 ok: EXECUTED, one manifest, {} outputs, none under {:?}",
        result.outputs.len(),
        profile.no_outputs_under
    );
    // Which observer produced these results, in the binary's own words.
    for message in ["observer", "run"] {
        if let Some(line) = last_event_line(&env.state(), message) {
            println!("A1 {message} event: {line}");
        }
    }

    // A2: second run on the same tree replays, byte-identical.
    let a2 = env.run_step(step, &[]);
    assert!(
        a2.hit() && a2.stderr.contains("inputs matched"),
        "A2: {}",
        a2.stderr
    );
    assert_eq!(a2.code, a1.code, "A2: exit code");
    assert_eq!(a2.stdout, a1.stdout, "A2: stdout bytes");
    println!(
        "A2 ok: REPLAYED, exit {} and {} stdout bytes identical",
        a2.code,
        a2.stdout.len()
    );

    let manifest = ledger().candidates(&key, 1).unwrap().remove(0);
    let recorded: BTreeSet<&str> = manifest.entries.iter().map(|e| e.p.as_str()).collect();
    let cas = Cas::open(env.state().join("cas")).unwrap();
    let tree = workspace_tree(&cas, &env.repo, &deps, profile);
    let sealed: usize = profile
        .seals(&tree)
        .iter()
        .map(|s| {
            manifest
                .entries
                .iter()
                .filter(|e| e.p == s.prefix || e.p.starts_with(&format!("{}/", s.prefix)))
                .count()
        })
        .sum();
    println!(
        "manifest: {} entries ({} f, {} d, {} a, {} l), {} under sealed prefixes {:?}",
        manifest.entries.len(),
        manifest
            .entries
            .iter()
            .filter(|e| e.k == Kind::File)
            .count(),
        manifest.entries.iter().filter(|e| e.k == Kind::Dir).count(),
        manifest
            .entries
            .iter()
            .filter(|e| e.k == Kind::Absent)
            .count(),
        manifest
            .entries
            .iter()
            .filter(|e| e.k == Kind::Symlink)
            .count(),
        sealed,
        profile
            .seals(&tree)
            .iter()
            .map(|s| s.prefix.clone())
            .collect::<Vec<_>>()
    );

    // A3: a commit touching only a docs file replays. README.md unless the
    // step reads it, else another unread markdown file, shallowest first.
    // (pytest resolves every file of every directory it collects from, so
    // on starlette README.md and docs/ are recorded inputs and a README-only
    // commit rightly misses; only .github/, a dot-directory, goes unread.)
    let mut markdown: Vec<&str> = tree
        .iter()
        .filter(|e| e.k == Kind::File && e.p.ends_with(".md") && !e.p.starts_with(".stillpass/"))
        .map(|e| e.p.as_str())
        .collect();
    markdown.sort_by_key(|p| (p.matches('/').count(), *p));
    let docs = std::iter::once("README.md")
        .chain(markdown)
        .find(|p| !recorded.contains(p) && env.repo.join(p).is_file())
        .expect("A3 precondition: the step reads every markdown file in the tree")
        .to_string();
    let text = std::fs::read_to_string(env.repo.join(&docs)).unwrap_or_default();
    env.write(&docs, &format!("{text}\nA3 edit\n"));
    env.commit("A3: docs only");
    let a3 = env.run_step(step, &[]);
    assert!(a3.hit(), "A3: {}", a3.stderr);
    println!("A3 ok: {docs}-only commit REPLAYED");

    // A4: a commit changing a source file the tests read misses and names it.
    // Preferably one in a subdirectory the step listed (A5 adds a file
    // there), so the edit is to the project's sources rather than a config
    // file at the top level (luxon's babel.config.js is also a read source).
    let listed: BTreeSet<&str> = manifest
        .entries
        .iter()
        .filter(|e| e.k == Kind::Dir)
        .map(|e| e.p.as_str())
        .collect();
    let editable = || {
        manifest
            .entries
            .iter()
            .filter(|e| e.k == Kind::File && profile.is_editable_source(&e.p))
    };
    let src_file = editable()
        .find(|e| {
            let (parent, _) = sp_cas::parent_and_name(&e.p);
            parent != "." && listed.contains(parent)
        })
        .or_else(|| editable().next())
        .unwrap_or_else(|| panic!("A4: no editable {tc} source file in the manifest"))
        .p
        .clone();
    let src = std::fs::read_to_string(env.repo.join(&src_file)).unwrap();
    env.write(&src_file, &format!("{src}\n{} A4 edit\n", profile.comment));
    env.commit("A4: touch a source file");
    let a4 = env.run_step(step, &[]);
    assert!(a4.executed(), "A4: {}", a4.stderr);
    assert!(
        a4.stderr.contains(&format!("differed at {src_file}")),
        "A4 must name {src_file}: {}",
        a4.stderr
    );
    println!("A4 ok: MISS, receipt names {src_file}");

    // A5: adding a file to a listed directory misses and names the listing.
    let dir = std::path::Path::new(&src_file)
        .parent()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let dir = if dir.is_empty() { ".".to_string() } else { dir };
    let newest = ledger().candidates(&key, 1).unwrap().remove(0);
    assert!(
        newest
            .entries
            .iter()
            .any(|e| e.p == dir && e.k == Kind::Dir),
        "A5 precondition: {dir} listed"
    );
    let added = child_path(&dir, "zz_stillpass_a5.txt");
    assert!(
        !newest.entries.iter().any(|e| e.p == added),
        "A5: the new name was probed"
    );
    env.write(&added, "new\n");
    env.commit("A5: add a file");
    let a5 = env.run_step(step, &[]);
    assert!(a5.executed(), "A5: {}", a5.stderr);
    assert!(
        a5.stderr.contains(&format!("differed at {dir}\n"))
            && a5.stderr.contains("directory listing changed"),
        "A5: {}",
        a5.stderr
    );
    println!("A5 ok: MISS, mismatch names the listing of {dir}");

    // A6: a changed env var is a different key with no manifests.
    let a6 = env.run_step(step, &["--env", "STILLPASS_A6=1"]);
    assert!(
        a6.executed() && a6.stderr.contains("no manifests for this step key"),
        "A6: {}",
        a6.stderr
    );
    println!("A6 ok: different key, no manifests");

    // A7: every recorded input, mutated, misses. Zero false hits.
    let tree = workspace_tree(&cas, &env.repo, &deps, profile);
    let cands = ledger().candidates(&key, sp_ledger::CANDIDATES).unwrap();
    let base = first_match(&cands, &tree).expect("A7: the current tree must match before mutating");
    let m = &cands[base];
    let t = Instant::now();
    let mut counts = [0usize; 4];
    let mut false_hits = Vec::new();
    for e in &m.entries {
        let (slot, mutated) = match e.k {
            Kind::File => {
                let mut bytes = cas.get(&e.h.unwrap()).unwrap().unwrap();
                match bytes.first_mut() {
                    Some(b) => *b ^= 0x01,
                    None => bytes.push(b'x'),
                }
                (
                    0,
                    tree.with_leaf_changed(Entry::file(
                        e.p.clone(),
                        Hash::of(&bytes),
                        e.m.unwrap(),
                    )),
                )
            }
            Kind::Dir => (
                1,
                tree.with_outputs(&[Entry::file(
                    child_path(&e.p, "zz_stillpass_a7"),
                    Hash::of(b"x"),
                    0o644,
                )]),
            ),
            Kind::Absent => (
                2,
                tree.with_outputs(&[Entry::file(e.p.clone(), Hash::of(b"x"), 0o644)]),
            ),
            Kind::Symlink => (
                3,
                tree.with_leaf_changed(Entry::symlink(e.p.clone(), "zz_stillpass_a7")),
            ),
            Kind::Deleted => unreachable!("manifests never hold x"),
        };
        counts[slot] += 1;
        if first_match(&cands, &mutated).is_some() {
            false_hits.push(format!("{} {:?}", e.p, e.k));
        }
    }
    println!(
        "A7: mutated {} f, {} d, {} a, {} l entries in {:.1}s: {} false hits",
        counts[0],
        counts[1],
        counts[2],
        counts[3],
        t.elapsed().as_secs_f64(),
        false_hits.len()
    );
    assert!(
        false_hits.is_empty(),
        "A7 false hits (the observer under-recorded): {false_hits:?}"
    );

    // A7b: paths the observer did not record are mutated on disk; the step
    // must replay, and a fresh execution must agree with the replay. Each
    // sample also compares what the fresh execution read with what the
    // replayed manifest recorded: the sets must agree, and neither may name
    // the mutated path. A real under-record looks exactly like a flaky test,
    // so on any disagreement both logs, both manifests and their diff are
    // written outside the temp state (STILLPASS_EVIDENCE_DIR, default
    // target/acceptance-evidence) before the assertion fires.
    // STILLPASS_A7B_EXTRA=path,path adds named unread files to the samples.
    let recorded: BTreeSet<&str> = m.entries.iter().map(|e| e.p.as_str()).collect();
    let unread: Vec<&Entry> = tree
        .iter()
        .filter(|e| e.k == Kind::File && !recorded.contains(e.p.as_str()))
        .filter(|e| !e.p.starts_with(".stillpass/"))
        .collect();
    let stride = (unread.len() / samples.max(1)).max(1);
    let extra = std::env::var("STILLPASS_A7B_EXTRA").unwrap_or_default();
    let picked: Vec<String> = unread
        .iter()
        .step_by(stride)
        .take(samples)
        .map(|e| e.p.clone())
        .chain(extra.split(',').filter(|s| !s.is_empty()).map(String::from))
        .collect();
    println!(
        "A7b: {} unread files in the tree, sampling {}",
        unread.len(),
        picked.len()
    );
    for (i, p) in picked.iter().enumerate() {
        assert!(
            !recorded.contains(p.as_str()) && env.repo.join(p).is_file(),
            "A7b: {p} must be an unread file of the tree"
        );
        // Corrupt it outright: if the step secretly read this file, its
        // result would certainly change (a cosmetic edit might not).
        std::fs::write(env.repo.join(p), b"\x00STILLPASS_A7B corrupted\n").unwrap();
        let hit = env.run_step(step, &[]);
        assert!(
            hit.hit(),
            "A7b: mutating unread {p} should replay: {}",
            hit.stderr
        );
        let fresh = env.run_step(step, &["--env", &format!("STILLPASS_A7B={i}")]);
        assert!(fresh.executed(), "A7b: {}", fresh.stderr);
        // What each run read: the manifest the replay matched, found the way
        // find_match finds it, and the manifest the fresh execution recorded.
        let now = workspace_tree(&cas, &env.repo, &deps, profile);
        let cands = ledger().candidates(&key, sp_ledger::CANDIDATES).unwrap();
        let hit_m = &cands[first_match(&cands, &now).expect("A7b: the replay matched a manifest")];
        let fresh_m = ledger().candidates(&newest_key(env), 1).unwrap().remove(0);
        let (hp, fp) = (paths(hit_m), paths(&fresh_m));
        // An absent probe one run made and the other did not is no evidence
        // of a missed read when the other run recorded the parent's listing:
        // that listing already pins the name as absent (the rule W4 uses).
        // pytest's cache provider probes a random `pytest-cache-files-*`
        // name in the workspace root on every run.
        let (implied, only_fresh, only_hit) = differing_paths(&fresh_m, hit_m);
        let read_mutated = fp.contains(p.as_str());
        let same_code = fresh.code == hit.code;
        let same_out = normalize(tc, &fresh.stdout) == normalize(tc, &hit.stdout);
        if same_code && same_out && !read_mutated && only_fresh.is_empty() && only_hit.is_empty() {
            println!(
                "A7b ok: unread {p} mutated, replay agrees with a fresh execution, both read the same {} paths{}",
                fp.len().min(hp.len()),
                if implied > 0 {
                    format!(
                        " ({implied} absent probes differ, each implied by the other run's listing)"
                    )
                } else {
                    String::new()
                }
            );
            continue;
        }
        let dir = save_evidence(
            &format!("a7b-{i}-{}", p.replace('/', "_")),
            &[
                ("mutated", p.as_bytes()),
                ("hit.stdout", &hit.stdout),
                ("hit.stderr", hit.stderr.as_bytes()),
                ("fresh.stdout", &fresh.stdout),
                ("fresh.stderr", fresh.stderr.as_bytes()),
                (
                    "hit.manifest.json",
                    hit_m.canonical_json().unwrap().as_bytes(),
                ),
                (
                    "fresh.manifest.json",
                    fresh_m.canonical_json().unwrap().as_bytes(),
                ),
                (
                    "manifest.diff",
                    only_fresh
                        .iter()
                        .map(|p| format!("only-fresh {p}\n"))
                        .chain(only_hit.iter().map(|p| format!("only-replayed {p}\n")))
                        .collect::<String>()
                        .as_bytes(),
                ),
            ],
        );
        let summary = format!(
            "evidence in {}; the fresh execution read {} paths, the replayed manifest {}; {} only fresh read, {} only the replayed run read; fresh read the mutated file: {read_mutated}",
            dir.display(),
            fp.len(),
            hp.len(),
            only_fresh.len(),
            only_hit.len()
        );
        assert!(
            !read_mutated,
            "A7b CRITICAL: the fresh execution read {p}, which the replayed manifest never recorded: the observer under-recorded. {summary}"
        );
        assert!(
            same_code,
            "A7b: {p} changed the exit code ({} fresh, {} replayed): an unrecorded read or a flaky test. {summary}\nthe step said:\n{}",
            fresh.code,
            hit.code,
            failures(&fresh.stdout)
        );
        assert!(
            same_out,
            "A7b: {p} changed the output: an unrecorded read or a flaky test. {summary}"
        );
        println!(
            "A7b WARNING: unread {p} mutated, results agree, but the two executions read different paths: {summary}; first: {:?} {:?}",
            only_fresh.first(),
            only_hit.first()
        );
    }

    if profile.warm.is_some() {
        warm_checks(env, step, &key, &src_file, profile);
    } else {
        println!("W: the {tc} profile has no warm state; W1-W4 do not apply");
    }

    println!(
        "acceptance: A1-A7{} pass under the {tc} profile{}",
        if profile.warm.is_some() { " and W" } else { "" },
        if real_repo { " on the spike repo" } else { "" }
    );
}

/// W: warm state (--warm). The build cache earlier warm runs left behind is
/// bound over the profile's cache dir, unobserved, on the claim that a
/// content-addressed cache cannot change a result. This audits the claim two
/// ways. Results: on the same tree and after a one-file change, a warm
/// execution must agree with a cold one on exit code and stdout up to
/// durations, and the warm key must replay. Inputs (W4): a warm run with a
/// full cache must read every path a run with the same env and an empty
/// cache read (W1 is that run), so the cache never hides an input from the
/// observer. W4 is what catches path-keyed caches (Go's module index, its
/// test-result cache); the profile's knobs turn them off.
fn warm_checks(env: &Env, step: &str, key: &Hash, src_file: &str, profile: &sp_key::Profile) {
    let ledger = || Ledger::open(&env.state().join("ledger.sqlite")).unwrap();
    let cold = env.run_step(step, &[]);
    assert!(cold.hit(), "W precondition: {}", cold.stderr);
    let w1 = env.run_step(step, &["--warm"]);
    assert!(
        w1.executed() && w1.stderr.contains("warm build cache"),
        "W1: {}",
        w1.stderr
    );
    assert_eq!(w1.code, cold.code, "W1: exit code cold vs warm");
    assert_eq!(
        normalize(profile.toolchain, &w1.stdout),
        normalize(profile.toolchain, &cold.stdout),
        "W1: stdout cold vs warm"
    );
    let warm_key = newest_key(env);
    assert_ne!(&warm_key, key, "W1: warm runs have their own step key");
    println!("W1 ok: first warm run (empty cache) agrees with the cold record");
    let src = std::fs::read_to_string(env.repo.join(src_file)).unwrap();
    env.write(src_file, &format!("{src}\n{} W edit\n", profile.comment));
    env.commit("W: one-file change");
    let c2 = env.run_step(step, &[]);
    let w2 = env.run_step(step, &["--warm"]);
    assert!(
        c2.executed() && w2.executed(),
        "W2: {}\n{}",
        c2.stderr,
        w2.stderr
    );
    assert_eq!(w2.code, c2.code, "W2: exit code cold vs warm");
    assert_eq!(
        normalize(profile.toolchain, &w2.stdout),
        normalize(profile.toolchain, &c2.stdout),
        "W2: stdout cold vs warm after a one-file change"
    );
    let w3 = env.run_step(step, &["--warm"]);
    assert!(w3.hit() && w3.stdout == w2.stdout, "W3: {}", w3.stderr);
    println!("W2 ok: warm agrees with cold after a one-file change; W3 ok: the warm key replays");

    // W4. An entry of the empty-cache manifest is covered by the full-cache
    // manifest if the path is there, or if it is an `a` whose parent is
    // there as a `d`: a listing that still matches implies the name is still
    // absent. (The assembler probes `#include "textflag.h"` beside each .s
    // file; with the asm action cached it never runs, but the directory it
    // probed is listed either way.) Anything else is an input the cache hid.
    // This is what found go test's cached vet verdict standing in for a read
    // of the package's ignored files; warm runs now over-record those.
    let warm = ledger().candidates(&warm_key, 2).unwrap();
    let (full, empty) = (&warm[0], &warm[1]);
    let full_dirs: BTreeSet<&str> = full
        .entries
        .iter()
        .filter(|e| e.k == Kind::Dir)
        .map(|e| e.p.as_str())
        .collect();
    let full_paths = paths(full);
    let (mut covered_by_listing, mut hidden) = (0, Vec::new());
    for e in &empty.entries {
        if full_paths.contains(e.p.as_str()) {
            continue;
        }
        let (parent, _) = sp_cas::parent_and_name(&e.p);
        if e.k == Kind::Absent && full_dirs.contains(parent) {
            covered_by_listing += 1;
        } else {
            hidden.push(format!("{} {:?}", e.p, e.k));
        }
    }
    println!(
        "W4: warm manifest has {} entries with a full cache, {} with an empty one; {} absent probes covered by their parent listing, {} hidden by the cache",
        full.entries.len(),
        empty.entries.len(),
        covered_by_listing,
        hidden.len()
    );
    assert!(
        hidden.is_empty(),
        "W4: the warm cache hid inputs from the observer: {:?}",
        hidden.iter().take(10).collect::<Vec<_>>()
    );
    let cold_m = &ledger().candidates(key, 1).unwrap()[0];
    let only_cold = cold_m
        .entries
        .iter()
        .filter(|e| !full_paths.contains(e.p.as_str()))
        .count();
    println!(
        "W4 ok; for the record, the cold env records {} paths the warm env does not (Go's module indexer walks whole modules; test caching stats every child of an opened directory)",
        only_cold
    );
}

fn paths(m: &sp_manifest::Manifest) -> BTreeSet<&str> {
    m.entries.iter().map(|e| e.p.as_str()).collect()
}

/// The paths only `a` recorded and only `b` recorded, leaving out absent
/// entries the other manifest implies (their parent directory is recorded
/// there as a listing). Returns (implied count, only in a, only in b).
fn differing_paths<'m>(
    a: &'m sp_manifest::Manifest,
    b: &'m sp_manifest::Manifest,
) -> (usize, Vec<&'m str>, Vec<&'m str>) {
    let listed = |m: &'m sp_manifest::Manifest| -> BTreeSet<&'m str> {
        m.entries
            .iter()
            .filter(|e| e.k == Kind::Dir)
            .map(|e| e.p.as_str())
            .collect()
    };
    let (pa, pb, la, lb) = (paths(a), paths(b), listed(a), listed(b));
    let mut implied = 0;
    let mut only =
        |m: &'m sp_manifest::Manifest, other: &BTreeSet<&str>, other_dirs: &BTreeSet<&str>| {
            let mut out = Vec::new();
            for e in &m.entries {
                if other.contains(e.p.as_str()) {
                    continue;
                }
                if e.k == Kind::Absent && other_dirs.contains(sp_cas::parent_and_name(&e.p).0) {
                    implied += 1;
                } else {
                    out.push(e.p.as_str());
                }
            }
            out.dedup();
            out
        };
    let only_a = only(a, &pb, &lb);
    let only_b = only(b, &pa, &la);
    (implied, only_a, only_b)
}

/// Writes evidence files where the temp state's deletion cannot reach them:
/// STILLPASS_EVIDENCE_DIR, else target/acceptance-evidence. Returns the dir.
fn save_evidence(name: &str, files: &[(&str, &[u8])]) -> std::path::PathBuf {
    let root = std::env::var_os("STILLPASS_EVIDENCE_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/acceptance-evidence")
        });
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    for (f, bytes) in files {
        std::fs::write(dir.join(f), bytes).unwrap();
    }
    dir
}

fn sqlite(env: &Env, sql: &str) -> String {
    let out = std::process::Command::new("sqlite3")
        .arg(env.state().join("ledger.sqlite"))
        .arg(sql)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// The step key of the newest manifest in the ledger.
fn newest_key(env: &Env) -> Hash {
    sqlite(
        env,
        "SELECT step_key FROM manifests ORDER BY created_at DESC, rowid DESC LIMIT 1",
    )
    .parse()
    .unwrap()
}

/// The run id of the newest result in the ledger.
fn newest_run(env: &Env) -> String {
    sqlite(
        env,
        "SELECT run_id FROM results ORDER BY created_at DESC, rowid DESC LIMIT 1",
    )
}

/// The tree `stillpass run` serves an observed step: the checkout without
/// `.git` and `.stillpass/`, with the profile's dependency trees (its seed
/// dirs under `.stillpass/`) grafted in from the deps dir. The same
/// construction as crates/sp-cli/src/deps.rs; A7 asserts the result matches
/// the recorded manifest before mutating anything, so a drift between the
/// two fails loudly instead of testing the wrong tree.
fn workspace_tree(cas: &Cas, repo: &Path, deps: &Path, profile: &sp_key::Profile) -> Tree {
    let mut tree = cas
        .import_dir(repo)
        .unwrap()
        .without_top_level(".git")
        .without_top_level(".stillpass");
    for name in profile
        .seed_dirs
        .iter()
        .filter_map(|d| d.strip_prefix(".stillpass/"))
    {
        let dir = deps.join(name);
        if dir.is_dir() {
            tree = tree.merged_at(
                &format!(".stillpass/{name}"),
                &cas.import_dir(&dir).unwrap(),
            );
        }
    }
    tree
}

fn only_key(env: &Env) -> Hash {
    let keys: Vec<String> = sqlite(env, "SELECT DISTINCT step_key FROM manifests")
        .lines()
        .map(String::from)
        .collect();
    assert_eq!(keys.len(), 1, "{keys:?}");
    keys[0].parse().unwrap()
}

/// The lines of a test log that name failures, for a panic message.
fn failures(stdout: &[u8]) -> String {
    String::from_utf8_lossy(stdout)
        .lines()
        .filter(|l| {
            l.starts_with("FAIL")
                || l.starts_with("--- FAIL")
                || l.contains("panic:")
                || l.starts_with("ERROR")
                || l.contains("Error:")
                || l.contains("failed")
        })
        .take(40)
        .collect::<Vec<_>>()
        .join("\n")
}

/// What a fresh execution may differ from the record in: timing tokens on
/// the lines the profile's test runner prints them on, and nothing else (no
/// line sorting, nothing dropped, so a reordered or extra line is a
/// difference). Go: go test's per-package `ok  \tpkg\t0.412s` and per-test
/// `--- FAIL: TestX (0.01s)`. Node: Jest's `Time:` line and `(N s)` / `(N ms)`
/// suffixes of PASS/FAIL and test lines. Python: pytest's summary line
/// (`==== 1414 passed in 6.74s ====`, padded to the terminal width).
fn normalize(tc: Toolchain, stdout: &[u8]) -> String {
    String::from_utf8_lossy(stdout)
        .lines()
        .map(|l| normalize_line(tc, l))
        .collect::<Vec<_>>()
        .join("\n")
}

fn normalize_line(tc: Toolchain, l: &str) -> String {
    match tc {
        Toolchain::Go => {
            if let Some((head, tail)) = l.rsplit_once('\t')
                && is_seconds(tail)
            {
                return format!("{head}\t<dur>");
            }
            let t = l.trim_start();
            if t.starts_with("--- ") {
                return paren_duration(l);
            }
            l.to_string()
        }
        Toolchain::Node => {
            let t = l.trim_start();
            if t.starts_with("Time:") {
                return format!("{}Time: <dur>", &l[..l.len() - t.len()]);
            }
            paren_duration(l)
        }
        Toolchain::Python => pytest_summary(l).unwrap_or_else(|| l.to_string()),
    }
}

/// `0.412s`, `12.5s`.
fn is_seconds(s: &str) -> bool {
    s.strip_suffix('s')
        .is_some_and(|n| !n.is_empty() && n.parse::<f64>().is_ok())
}

/// A trailing `(0.01s)`, `(5.526 s)` or `(12 ms)` becomes `(<dur>)`.
fn paren_duration(l: &str) -> String {
    if let Some(open) = l.rfind(" (")
        && let Some(inner) = l[open + 2..].strip_suffix(')')
    {
        let n = inner
            .strip_suffix("ms")
            .or_else(|| inner.strip_suffix('s'))
            .map(str::trim_end);
        if n.is_some_and(|n| !n.is_empty() && n.parse::<f64>().is_ok()) {
            return format!("{} (<dur>)", &l[..open]);
        }
    }
    l.to_string()
}

/// pytest's final line, `===== 1414 passed, 2 xfailed in 6.74s =====` (with an
/// optional ` (0:01:02)`), whose `=` fill depends on the duration's length.
fn pytest_summary(l: &str) -> Option<String> {
    let inner = l
        .strip_prefix('=')?
        .strip_suffix('=')?
        .trim_matches('=')
        .trim();
    let (what, when) = inner.rsplit_once(" in ")?;
    let secs = when.split_once(' ').map_or(when, |(s, _)| s);
    is_seconds(secs).then(|| format!("== {what} in <dur> =="))
}

#[cfg(test)]
mod normalize_tests {
    use super::*;

    #[test]
    fn only_runner_timing_tokens() {
        let go = |l| normalize_line(Toolchain::Go, l);
        assert_eq!(go("ok  \tpkg/cmd\t0.412s"), "ok  \tpkg/cmd\t<dur>");
        assert_eq!(
            go("    --- FAIL: TestX (0.01s)"),
            "    --- FAIL: TestX (<dur>)"
        );
        assert_eq!(go("ok  \tpkg/cmd\t(cached)"), "ok  \tpkg/cmd\t(cached)");
        assert_eq!(go("took 0.4s here"), "took 0.4s here");
        let node = |l| normalize_line(Toolchain::Node, l);
        assert_eq!(node("Time:        1.263 s, estimated 2 s"), "Time: <dur>");
        assert_eq!(
            node("PASS test/interval/setter.test.js (18.763 s)"),
            "PASS test/interval/setter.test.js (<dur>)"
        );
        assert_eq!(node("  ✓ Diff (DST) (1 ms)"), "  ✓ Diff (DST) (<dur>)");
        assert_eq!(node("  ✓ Diff (DST)"), "  ✓ Diff (DST)");
        assert_eq!(node("All files |   98.1 |"), "All files |   98.1 |");
        let py = |l| normalize_line(Toolchain::Python, l);
        assert_eq!(
            py("======= 1414 passed, 2 xfailed in 6.74s ========"),
            py("======= 1414 passed, 2 xfailed in 10.66s =======")
        );
        assert_eq!(
            py("== 3 failed, 1 passed in 61.20s (0:01:01) =="),
            "== 3 failed, 1 passed in <dur> =="
        );
        assert_eq!(
            py("== 1414 passed in 6.74s =="),
            "== 1414 passed in <dur> =="
        );
        assert_eq!(
            py("tests/test_a.py ....  [ 50%]"),
            "tests/test_a.py ....  [ 50%]"
        );
        assert_eq!(
            py("== 1413 passed in 6.74s =="),
            "== 1413 passed in <dur> =="
        );
        // Nothing is sorted or dropped: order and date lines are compared.
        let a = b"PASS b.test.js (2 s)\nPASS a.test.js (1 s)\n";
        let b = b"PASS a.test.js (4 s)\nPASS b.test.js (1 s)\n";
        assert_ne!(normalize(Toolchain::Node, a), normalize(Toolchain::Node, b));
        let d1 = b"Sat Oct 31 11:13:24 GMT 2026\nok\n";
        let d2 = b"Sat Oct 31 11:13:25 GMT 2026\nok\n";
        assert_ne!(
            normalize(Toolchain::Node, d1),
            normalize(Toolchain::Node, d2)
        );
    }
}
