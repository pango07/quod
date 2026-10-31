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
    // must replay, and a fresh execution must agree with the replay.
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
    for (i, e) in unread.iter().step_by(stride).take(samples).enumerate() {
        let path = env.repo.join(&e.p);
        // Corrupt it outright: if the step secretly read this file, its
        // result would certainly change (a cosmetic edit might not).
        std::fs::write(&path, b"\x00STILLPASS_A7B corrupted\n").unwrap();
        let hit = env.run_step(step, &[]);
        assert!(
            hit.hit(),
            "A7b: mutating unread {} should replay: {}",
            e.p,
            hit.stderr
        );
        let fresh = env.run_step(step, &["--env", &format!("STILLPASS_A7B={i}")]);
        assert!(fresh.executed());
        assert_eq!(
            fresh.code, hit.code,
            "A7b: {} changed the exit code; the observer missed a read",
            e.p
        );
        assert_eq!(
            normalize(&fresh.stdout),
            normalize(&hit.stdout),
            "A7b: {} changed the output; the observer missed a read",
            e.p
        );
        println!(
            "A7b ok: unread {} mutated, replay agrees with a fresh execution",
            e.p
        );
    }
    println!(
        "acceptance: A1-A7 pass{}",
        if spike.is_some() {
            " on the spike repo"
        } else {
            ""
        }
    );
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
