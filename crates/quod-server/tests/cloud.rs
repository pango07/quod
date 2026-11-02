//! Quod Cloud's backends against a real Postgres and a real S3-compatible
//! store (CI: the `server-cloud` job's postgres service and MinIO). Every
//! test returns at once without `QUOD_TEST_DATABASE_URL` (see
//! `common::cloud`). The whole of tests/server.rs (S1 to S15, the S2
//! races among them) also runs on these backends in that job.
//!
//! P1 migrations: versioned, one transaction, idempotent, safe when
//!    instances start at once; a newer database is refused
//! P2 `atomically`: all or nothing, and racing read-modify-writes of one
//!    scope never interleave (no lost update); scopes and teams apart
//! P3 lookups (`find_match`) answer exactly as SQLite's on the same history
//! P4 blobs: content-addressed, per team, multipart, streamed, checked on read
//! P5 a SQLite data dir imports (once) and a fresh runner replays from it

mod common;

use std::io::Read;

use common::*;
use quod_cas::{BlobStore, CasError, Hash, Tree};
use quod_ledger::{AuditRow, Ledger, LedgerError, LedgerStore, RunFacts, TrustRow};
use quod_manifest::StepResult;
use quod_server::pg::{PG_SCHEMA_VERSION, Pg};
use quod_server::s3::S3Blobs;

const CMD: &str = "go test ./...";

fn audit(key: Hash) -> AuditRow {
    AuditRow {
        id: 0,
        step_key: key,
        manifest_id: Hash::of(b"m"),
        at: String::new(),
        phase: "audit".into(),
        diverged: false,
        expected_run: None,
        actual_run: None,
        detail: None,
    }
}

#[test]
fn p1_migrations_are_versioned_transactional_and_locked() {
    if !cloud() {
        return;
    }
    let env = cloud_env();
    // Four instances start at once on an empty database: one migrates,
    // the others wait on the advisory lock and find the work done.
    let pgs: Vec<Result<Pg, String>> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..4).map(|_| s.spawn(|| Pg::connect(&env.pg))).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    for p in &pgs {
        assert!(p.is_ok(), "{:?}", p.as_ref().err());
    }
    let pg = pgs.into_iter().next().unwrap().unwrap();
    assert_eq!(pg.schema_version().unwrap(), PG_SCHEMA_VERSION);
    let mut c = pg.conn().unwrap();
    let applied: i64 = c
        .query_one("SELECT count(*) FROM quod_schema", &[])
        .unwrap()
        .get(0);
    assert_eq!(applied, PG_SCHEMA_VERSION);
    // Again: nothing to do.
    assert_eq!(
        pg.migrate().unwrap(),
        (PG_SCHEMA_VERSION, PG_SCHEMA_VERSION)
    );
    // A database a newer server migrated is refused, not used.
    c.execute(
        "INSERT INTO quod_schema (version, applied_at) VALUES (99, 'later')",
        &[],
    )
    .unwrap();
    let e = Pg::connect(&env.pg).err().unwrap();
    assert!(e.contains("newer"), "{e}");
}

#[test]
fn p2_atomically_is_all_or_nothing_and_never_interleaves() {
    if !cloud() {
        return;
    }
    let cloud = fresh_cloud();
    let id = cloud
        .pg
        .scope_id("acme", "app-1", "acme/app", "trusted", true)
        .unwrap()
        .unwrap();
    let key = key_of("p2");
    let l = cloud.pg.ledger("acme", id).unwrap();

    // All or nothing: a failure inside rolls back every write.
    let r: Result<(), LedgerError> = l.atomically(|l| {
        l.put_trust(&TrustRow::new(key, Some("x")))?;
        l.record_audit(&audit(key))?;
        Err(LedgerError::Corrupt("boom".into()))
    });
    assert!(r.is_err());
    assert!(l.trust(&key).unwrap().is_none());
    assert!(l.audits(None, 10).unwrap().is_empty());

    // Eight writers, ten read-modify-writes each, a pause between the
    // read and the write: every increment lands (no writer interleaves).
    std::thread::scope(|s| {
        for _ in 0..8 {
            s.spawn(|| {
                let l = cloud.pg.ledger("acme", id).unwrap();
                for _ in 0..10 {
                    l.atomically(|l| -> Result<(), LedgerError> {
                        let mut t = l.trust(&key)?.unwrap_or_else(|| TrustRow::new(key, None));
                        std::thread::sleep(std::time::Duration::from_millis(2));
                        t.eligible += 1;
                        l.put_trust(&t)
                    })
                    .unwrap();
                }
            });
        }
    });
    assert_eq!(l.trust(&key).unwrap().unwrap().eligible, 80);

    // Another team's scope with the same names is its own.
    let other = cloud
        .pg
        .scope_id("beta", "app-1", "acme/app", "trusted", true)
        .unwrap()
        .unwrap();
    assert_ne!(other, id);
    let b = cloud.pg.ledger("beta", other).unwrap();
    assert!(b.trust(&key).unwrap().is_none());
    // A scope id of another team reads nothing through this team.
    let wrong = cloud.pg.ledger("beta", id).unwrap();
    assert!(wrong.trust(&key).unwrap().is_none());
    // A nested `atomically` joins the outer transaction.
    l.atomically(|l| {
        l.atomically(|l| l.put_trust(&TrustRow::new(key_of("nested"), None)))
    })
    .unwrap();
    assert!(l.trust(&key_of("nested")).unwrap().is_some());
}

fn result_for(key: Hash, mid: Hash, run: &str, out: &str) -> StepResult {
    StepResult {
        v: quod_manifest::VERSION,
        step_key: key,
        manifest_id: mid,
        run_id: run.into(),
        recorded_at: quod_ledger::now_rfc3339(),
        exit_code: 0,
        stdout: Hash::of(out.as_bytes()),
        stderr: Hash::of(b""),
        outputs: vec![],
        root_outputs: vec![],
        duration_ms: 10,
        commands: vec![],
    }
}

/// What a lookup found, comparable across stores.
fn summary(l: &quod_ledger::Lookup) -> String {
    format!(
        "hit {:?} candidates {} closest {:?}",
        l.hit
            .as_ref()
            .map(|(m, r)| (quod_manifest::manifest_id(m).unwrap(), r.run_id.clone())),
        l.candidates,
        l.closest
    )
}

#[test]
fn p3_lookups_match_sqlite_exactly() {
    if !cloud() {
        return;
    }
    let cloud = fresh_cloud();
    let id = cloud
        .pg
        .scope_id("acme", "app-1", "acme/app", "trusted", true)
        .unwrap()
        .unwrap();
    let pg = cloud.pg.ledger("acme", id).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let sq = Ledger::open(&dir.path().join("ledger.sqlite")).unwrap();
    let key = key_of("p3");
    let trees: Vec<Tree> = (0..6)
        .map(|i| {
            let b = format!("v{}", i % 3);
            let d = format!("w{}", i / 2);
            tree(&[
                ("go.mod", "m"),
                ("a/b.go", b.as_str()),
                ("c/d.go", d.as_str()),
                ("c/e/f.go", "f"),
            ])
        })
        .collect();
    // A history: five manifests, then the first again (newest once more).
    let mut n = 0;
    for i in [0, 1, 2, 3, 4, 0] {
        let m = manifest(key, &trees[i]);
        let mid = quod_manifest::manifest_id(&m).unwrap();
        n += 1;
        let run = format!("run-{n}");
        let r = result_for(key, mid, &run, &format!("out {i}"));
        let facts = RunFacts {
            id: &run,
            step_key: Some(&key),
            ..RunFacts::default()
        };
        sq.record_execution_facts(&facts, &m, &r).unwrap();
        LedgerStore::record_execution_facts(&pg, &facts, &m, &r).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
    }
    let mut probes = trees.clone();
    probes.push(tree(&[("go.mod", "other")]));
    probes.push(tree(&[]));
    for (i, t) in probes.iter().enumerate() {
        let a = sq.find_match(&key, t).unwrap();
        let b = LedgerStore::find_match(&pg, &key, t).unwrap();
        assert_eq!(summary(&a), summary(&b), "probe {i}");
    }
    // Both found something and missed something.
    assert!(sq.find_match(&key, &trees[3]).unwrap().hit.is_some());
    assert!(sq.find_match(&key, &trees[5]).unwrap().hit.is_none());
    // An unknown key: no candidates either way.
    let other = key_of("nothing");
    assert_eq!(
        summary(&sq.find_match(&other, &trees[0]).unwrap()),
        summary(&LedgerStore::find_match(&pg, &other, &trees[0]).unwrap())
    );
}

#[test]
fn p4_blobs_are_content_addressed_per_team_and_streamed() {
    if !cloud() {
        return;
    }
    let cloud = fresh_cloud();
    let a = S3Blobs::new(cloud.s3.clone(), "acme");
    let b = S3Blobs::new(cloud.s3.clone(), "beta");
    let small = b"hello".to_vec();
    let h = a.put_blob(&small).unwrap();
    assert_eq!(h, Hash::of(&small));
    assert!(a.object(&h).ends_with(&format!("teams/acme/blobs/{}", h.hex())));
    assert!(a.has_blob(&h).unwrap());
    assert!(!b.has_blob(&h).unwrap());
    assert_eq!(a.read_blob(&h).unwrap(), small);
    assert!(matches!(b.read_blob(&h), Err(CasError::Missing(m)) if m == h));

    // Large: three parts of 5 MiB (the test part size), from a file and
    // from memory.
    let big: Vec<u8> = (0..(12usize << 20) + 7)
        .map(|i| (i * 31 % 251) as u8)
        .collect();
    let f = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(f.path(), &big).unwrap();
    let (hb, sent) = a.put_blob_file(f.path()).unwrap();
    assert_eq!((hb, sent), (Hash::of(&big), big.len() as u64));
    assert_eq!(a.size(&hb).unwrap(), Some(big.len() as u64));
    let mut back = vec![];
    a.open_blob_reader(&hb)
        .unwrap()
        .read_to_end(&mut back)
        .unwrap();
    assert!(back == big);
    // Stored once: a second put sends nothing.
    assert_eq!(a.put_blob_file(f.path()).unwrap(), (hb, 0));
    let mut big2 = big.clone();
    big2[0] ^= 1;
    let h2 = b.put_blob(&big2).unwrap();
    assert_eq!(h2, Hash::of(&big2));
    assert!(b.read_blob(&h2).unwrap() == big2);

    // Bytes under a name that is not their hash (written around the
    // store) are refused when read, whole or streamed.
    let named = Hash::of(b"what the name says");
    cloud.s3.put(&a.object(&named), b"something else").unwrap();
    assert!(a.read_blob(&named).is_err());
    let mut sink = vec![];
    assert!(
        a.open_blob_reader(&named)
            .unwrap()
            .read_to_end(&mut sink)
            .is_err()
    );
}

fn files() -> Vec<(&'static str, &'static str)> {
    vec![("go.mod", "module x"), ("main.go", "package main")]
}

/// Every file under `dir`, with its bytes.
fn snapshot(dir: &std::path::Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    let mut out = vec![];
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push((p.clone(), std::fs::read(&p).unwrap()));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn p5_a_sqlite_data_dir_imports_and_replays_from_postgres() {
    if !cloud() {
        return;
    }
    let t = tree(&files());
    // A self-hosted server proves a step.
    let s = Server::start_backend(document(1), None, None, tempfile::tempdir().unwrap());
    let mut r1 = Runner::token(&s, ACME_RUNNER);
    r1.policy = policy(1, 0.0);
    assert!(proven(&step(&r1, CMD, &t, "from sqlite\n")));
    let data = s.stop();
    let dir = data.path().join("data");
    let before = snapshot(&dir.join("teams"));

    // Imported once; a second import skips the team.
    let cloud = fresh_cloud();
    let i = quod_server::import::import_sqlite(&dir, &cloud).unwrap();
    assert_eq!(i.teams, vec!["acme".to_string()]);
    assert!(i.blobs >= 2 && i.damaged_blobs.is_empty(), "{i:?}");
    assert_eq!(i.rows.get("results"), Some(&1), "{i:?}");
    assert_eq!(i.rows.get("manifests"), Some(&1), "{i:?}");
    assert!(i.rows.get("steps").is_some_and(|n| *n >= 1), "{i:?}");
    let again = quod_server::import::import_sqlite(&dir, &cloud).unwrap();
    assert_eq!(again.skipped, vec!["acme".to_string()]);
    assert!(again.teams.is_empty());
    assert!(
        quod_server::import::not_imported(&dir, &cloud)
            .unwrap()
            .is_empty()
    );
    // The volume's files are as they were.
    assert!(snapshot(&dir.join("teams")) == before);

    // A server on Postgres and the bucket replays the proof to a fresh
    // runner, and shows the imported steps.
    let s2 = Server::start_backend(document(1), None, Some(cloud.clone()), data);
    let mut r2 = Runner::token(&s2, ACME_RUNNER);
    r2.policy = policy(1, 0.0);
    match claim(&r2, CMD, &t) {
        Outcome::Replayed(out) => assert_eq!(out, b"from sqlite\n"),
        o => panic!("expected a replay of the imported proof: {o:?}"),
    }
    let (code, body) = s2.get("/v1/teams/acme/steps", ACME_VIEWER);
    assert_eq!(code, 200, "{body}");
    let data = s2.stop();

    // Postgres that already holds a team's scopes (a server ran on it
    // first) is never merged into.
    let cloud2 = fresh_cloud();
    let s3 = Server::start_backend(
        document(1),
        None,
        Some(cloud2.clone()),
        tempfile::tempdir().unwrap(),
    );
    let mut r3 = Runner::token(&s3, ACME_RUNNER);
    r3.policy = policy(1, 0.0);
    step(&r3, CMD, &t, "on postgres\n");
    drop(s3.stop());
    let e = quod_server::import::import_sqlite(&data.path().join("data"), &cloud2)
        .err()
        .unwrap();
    assert!(format!("{e:#}").contains("does not merge"), "{e:#}");
}
