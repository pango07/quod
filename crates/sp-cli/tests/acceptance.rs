//! The spike's acceptance criteria A1 to A7, in order, on one checkout.
//!
//! Runs on the tiny Go module by default (every PR), or on the spike repo
//! when STILLPASS_SPIKE_REPO is set (scripts/acceptance.sh, nightly). The
//! repo is copied first; the original is never touched.
//!
//! A7b goes beyond the handoff: A7 mutates only paths the manifest
//! recorded, so it proves the matcher, not the observer. A7b mutates paths
//! the observer did NOT record, expects a hit (the model says they are not
//! inputs), and re-executes the step under a fresh key to check the result
//! really is unchanged. An unlogged read would show up as a difference.

mod common;
use std::collections::BTreeSet;
use std::path::Path;
use std::time::Instant;

use common::*;
use sp_cas::{Cas, Entry, Hash, Kind, child_path};
use sp_ledger::{Ledger, first_match};

const STEP_DEFAULT: &str = "go test ./...";

#[test]
fn acceptance() {
    if !enabled() {
        return;
    }
    let spike = std::env::var("STILLPASS_SPIKE_REPO").ok();
    let env = match &spike {
        Some(src) => Env::from_copy(Path::new(src)),
        None => Env::new(),
    };
    let step = std::env::var("STILLPASS_STEP").unwrap_or_else(|_| STEP_DEFAULT.into());
    let step = step.as_str();
    let ledger = || Ledger::open(&env.state().join("ledger.sqlite")).unwrap();

    // A1: first run executes and records one manifest and result.
    let a1 = env.run_step(step, &[]);
    assert!(a1.executed() && a1.code == 0, "A1: {}", a1.stderr);
    let key = only_key(&env);
    assert_eq!(
        ledger().manifest_count(&key).unwrap(),
        1,
        "A1: one manifest"
    );
    let inspect = env.stillpass(&["inspect", &key.to_string()]);
    assert!(String::from_utf8_lossy(&inspect.stdout).contains("1 manifest(s)"));
    println!("A1 ok: EXECUTED, one manifest");

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

    // A3: a commit touching only README.md replays.
    assert!(
        !recorded.contains("README.md"),
        "A3 precondition: the step reads README.md, so A3 cannot hold on this repo"
    );
    let readme = std::fs::read_to_string(env.repo.join("README.md")).unwrap_or_default();
    env.write("README.md", &format!("{readme}\nA3 edit\n"));
    env.commit("A3: README only");
    let a3 = env.run_step(step, &[]);
    assert!(a3.hit(), "A3: {}", a3.stderr);
    println!("A3 ok: README-only commit REPLAYED");

    // A4: a commit changing a Go file the tests read misses and names it.
    let go_file = manifest
        .entries
        .iter()
        .find(|e| {
            e.k == Kind::File
                && e.p.ends_with(".go")
                && !e.p.starts_with(".stillpass/")
                && !e.p.ends_with("_test.go")
        })
        .expect("A4: no non-test .go file in the manifest")
        .p
        .clone();
    let src = std::fs::read_to_string(env.repo.join(&go_file)).unwrap();
    env.write(&go_file, &format!("{src}\n// A4 edit\n"));
    env.commit("A4: touch a Go file");
    let a4 = env.run_step(step, &[]);
    assert!(a4.executed(), "A4: {}", a4.stderr);
    assert!(
        a4.stderr.contains(&format!("differed at {go_file}")),
        "A4 must name {go_file}: {}",
        a4.stderr
    );
    println!("A4 ok: MISS, receipt names {go_file}");

    // A5: adding a file to a listed directory misses and names the listing.
    let dir = std::path::Path::new(&go_file)
        .parent()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
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
    let cas = Cas::open(env.state().join("cas")).unwrap();
    let tree = cas.import_dir(&env.repo).unwrap().without_top_level(".git");
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
    let samples: usize = std::env::var("STILLPASS_A7B_SAMPLES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
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
        let now = cas.import_dir(&env.repo).unwrap().without_top_level(".git");
        let cands = ledger().candidates(&key, sp_ledger::CANDIDATES).unwrap();
        let hit_m = &cands[first_match(&cands, &now).expect("A7b: the replay matched a manifest")];
        let fresh_m = ledger().candidates(&newest_key(&env), 1).unwrap().remove(0);
        let (hp, fp) = (paths(hit_m), paths(&fresh_m));
        let only_fresh: Vec<&str> = fp.difference(&hp).copied().collect();
        let only_hit: Vec<&str> = hp.difference(&fp).copied().collect();
        let read_mutated = fp.contains(p.as_str());
        let same_code = fresh.code == hit.code;
        let same_out = normalize(&fresh.stdout) == normalize(&hit.stdout);
        if same_code && same_out && !read_mutated && only_fresh.is_empty() && only_hit.is_empty() {
            println!(
                "A7b ok: unread {p} mutated, replay agrees with a fresh execution, both read the same {} paths",
                fp.len()
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
            "A7b: {p} changed the exit code ({} fresh, {} replayed): an unrecorded read or a flaky test. {summary}\ngo test said:\n{}",
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
    // W: warm state (--warm). The build cache earlier warm runs left behind
    // is bound over GOCACHE, unobserved, on the claim that a content-addressed
    // cache cannot change a result. This audits the claim two ways. Results:
    // on the same tree and after a one-file change, a warm execution must
    // agree with a cold one on exit code and stdout up to durations, and the
    // warm key must replay. Inputs (W4): a warm run with a full cache must
    // read every path a run with the same env and an empty cache read (W1 is
    // that run), so the cache never hides an input from the observer. W4 is
    // what catches Go's path-keyed caches (the module index, the test-result
    // cache); the warm env turns both off.
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
        normalize(&w1.stdout),
        normalize(&cold.stdout),
        "W1: stdout cold vs warm"
    );
    let warm_key = newest_key(&env);
    assert_ne!(warm_key, key, "W1: warm runs have their own step key");
    println!("W1 ok: first warm run (empty cache) agrees with the cold record");
    let src = std::fs::read_to_string(env.repo.join(&go_file)).unwrap();
    env.write(&go_file, &format!("{src}\n// W edit\n"));
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
        normalize(&w2.stdout),
        normalize(&c2.stdout),
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
    let cold_m = &ledger().candidates(&key, 1).unwrap()[0];
    let only_cold = cold_m
        .entries
        .iter()
        .filter(|e| !full_paths.contains(e.p.as_str()))
        .count();
    println!(
        "W4 ok; for the record, the cold env records {} paths the warm env does not (Go's module indexer walks whole modules; test caching stats every child of an opened directory)",
        only_cold
    );

    println!(
        "acceptance: A1-A7 and W pass{}",
        if spike.is_some() {
            " on the spike repo"
        } else {
            ""
        }
    );
}

fn paths(m: &sp_manifest::Manifest) -> BTreeSet<&str> {
    m.entries.iter().map(|e| e.p.as_str()).collect()
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

/// The step key of the newest manifest in the ledger.
fn newest_key(env: &Env) -> Hash {
    let out = std::process::Command::new("sqlite3")
        .arg(env.state().join("ledger.sqlite"))
        .arg("SELECT step_key FROM manifests ORDER BY created_at DESC, rowid DESC LIMIT 1")
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
}

fn only_key(env: &Env) -> Hash {
    let out = std::process::Command::new("sqlite3")
        .arg(env.state().join("ledger.sqlite"))
        .arg("SELECT DISTINCT step_key FROM manifests")
        .output()
        .unwrap();
    let keys: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(String::from)
        .collect();
    assert_eq!(keys.len(), 1, "{keys:?}");
    keys[0].parse().unwrap()
}

/// The lines of a go test log that name failures, for a panic message.
fn failures(stdout: &[u8]) -> String {
    String::from_utf8_lossy(stdout)
        .lines()
        .filter(|l| l.starts_with("FAIL") || l.starts_with("--- FAIL") || l.contains("panic:"))
        .take(40)
        .collect::<Vec<_>>()
        .join("\n")
}

/// go test prints per-package durations; a fresh run differs only there.
fn normalize(stdout: &[u8]) -> String {
    String::from_utf8_lossy(stdout)
        .lines()
        .map(|l| match l.rsplit_once('\t') {
            Some((head, tail))
                if tail.ends_with('s') && tail[..tail.len() - 1].parse::<f64>().is_ok() =>
            {
                format!("{head}\t<dur>")
            }
            _ => l.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}
