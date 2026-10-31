//! SQLite ledger: manifests and results by step key, runs, and the blob
//! index. Single writer, WAL mode. Blobs live in the CAS; the `blobs` table
//! is an index, not the store.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};
use sp_cas::{Hash, Tree};
use sp_manifest::{Manifest, Mismatch, StepResult, manifest_id, matches};

/// How many candidate manifests a lookup verifies, newest first.
pub const CANDIDATES: usize = 50;

const MIGRATIONS: &[&str] = &[
    // v1: the Schemas section.
    "CREATE TABLE runs      (id TEXT PRIMARY KEY, started_at TEXT NOT NULL, commit_sha TEXT, host TEXT);
     CREATE TABLE manifests (id TEXT PRIMARY KEY, step_key TEXT NOT NULL, run_id TEXT NOT NULL, created_at TEXT NOT NULL, json BLOB NOT NULL);
     CREATE INDEX manifests_by_key ON manifests (step_key, created_at DESC);
     CREATE TABLE results   (id TEXT PRIMARY KEY, step_key TEXT NOT NULL, manifest_id TEXT NOT NULL, run_id TEXT NOT NULL, created_at TEXT NOT NULL, json BLOB NOT NULL);
     CREATE INDEX results_by_key ON results (step_key, manifest_id, created_at DESC);
     CREATE TABLE blobs     (hash TEXT PRIMARY KEY, size INTEGER NOT NULL, path TEXT NOT NULL);",
];

#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    #[error("sqlite: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("stored JSON is corrupt: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Manifest(#[from] sp_manifest::ManifestError),
    #[error("ledger schema version {found} is newer than this binary ({known})")]
    TooNew { found: usize, known: usize },
}

pub struct Ledger {
    db: Connection,
}

/// The outcome of a lookup.
#[derive(Debug)]
pub struct Lookup {
    /// The first candidate that matches the tree, with its newest result.
    pub hit: Option<(Manifest, StepResult)>,
    /// How many manifests the key had (up to CANDIDATES were checked).
    pub candidates: usize,
    /// On a miss with candidates: the newest candidate's first mismatch and
    /// that candidate's id.
    pub closest: Option<(Hash, Mismatch)>,
}

/// One manifest stored for a key, for `inspect`.
#[derive(Debug, Clone)]
pub struct ManifestRow {
    pub id: Hash,
    pub run_id: String,
    pub created_at: String,
    pub entries: usize,
}

#[derive(Debug, Clone)]
pub struct RunRow {
    pub id: String,
    pub started_at: String,
    pub commit_sha: Option<String>,
    pub host: Option<String>,
}

impl Ledger {
    pub fn open(path: &Path) -> Result<Ledger, LedgerError> {
        let db = Connection::open(path)?;
        db.pragma_update(None, "journal_mode", "WAL")?;
        db.pragma_update(None, "synchronous", "NORMAL")?;
        let mut l = Ledger { db };
        l.migrate()?;
        Ok(l)
    }

    pub fn open_in_memory() -> Result<Ledger, LedgerError> {
        let mut l = Ledger {
            db: Connection::open_in_memory()?,
        };
        l.migrate()?;
        Ok(l)
    }

    fn migrate(&mut self) -> Result<(), LedgerError> {
        let v = self
            .db
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))?
            as usize;
        if v > MIGRATIONS.len() {
            return Err(LedgerError::TooNew {
                found: v,
                known: MIGRATIONS.len(),
            });
        }
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(v) {
            let tx = self.db.transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", (i + 1) as i64)?;
            tx.commit()?;
        }
        Ok(())
    }

    pub fn schema_version(&self) -> Result<usize, LedgerError> {
        Ok(self
            .db
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))? as usize)
    }

    pub fn record_run(
        &self,
        id: &str,
        commit_sha: Option<&str>,
        host: Option<&str>,
    ) -> Result<(), LedgerError> {
        self.db.execute(
            "INSERT INTO runs (id, started_at, commit_sha, host) VALUES (?1, ?2, ?3, ?4)",
            params![id, now_rfc3339(), commit_sha, host],
        )?;
        Ok(())
    }

    pub fn run(&self, id: &str) -> Result<Option<RunRow>, LedgerError> {
        Ok(self
            .db
            .query_row(
                "SELECT id, started_at, commit_sha, host FROM runs WHERE id = ?1",
                [id],
                |r| {
                    Ok(RunRow {
                        id: r.get(0)?,
                        started_at: r.get(1)?,
                        commit_sha: r.get(2)?,
                        host: r.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    /// Stores a manifest and returns its id. Re-recording an identical
    /// manifest refreshes its position as newest.
    pub fn put_manifest(&self, m: &Manifest, run_id: &str) -> Result<Hash, LedgerError> {
        let id = manifest_id(m)?;
        self.db.execute(
            // REPLACE gives the row a new rowid, which breaks created_at ties.
            "INSERT OR REPLACE INTO manifests (id, step_key, run_id, created_at, json) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id.to_string(), m.step_key.to_string(), run_id, now_rfc3339(), m.canonical_json()?.into_bytes()],
        )?;
        Ok(id)
    }

    pub fn put_result(&self, r: &StepResult) -> Result<(), LedgerError> {
        let json = sp_cas::canonical_json(r).map_err(sp_manifest::ManifestError::from)?;
        self.db.execute(
            "INSERT INTO results (id, step_key, manifest_id, run_id, created_at, json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![r.run_id, r.step_key.to_string(), r.manifest_id.to_string(), r.run_id, now_rfc3339(), json.into_bytes()],
        )?;
        Ok(())
    }

    pub fn put_blob(&self, h: &Hash, size: u64, path: &str) -> Result<(), LedgerError> {
        self.db.execute(
            "INSERT OR IGNORE INTO blobs (hash, size, path) VALUES (?1, ?2, ?3)",
            params![h.to_string(), size as i64, path],
        )?;
        Ok(())
    }

    /// Up to `limit` manifests for `step_key`, newest first.
    pub fn candidates(&self, step_key: &Hash, limit: usize) -> Result<Vec<Manifest>, LedgerError> {
        let mut st = self.db.prepare_cached(
            "SELECT json FROM manifests WHERE step_key = ?1 ORDER BY created_at DESC, rowid DESC LIMIT ?2",
        )?;
        let rows = st.query_map(params![step_key.to_string(), limit as i64], |r| {
            r.get::<_, Vec<u8>>(0)
        })?;
        rows.map(|j| Ok(serde_json::from_slice(&j?)?)).collect()
    }

    pub fn manifest_count(&self, step_key: &Hash) -> Result<usize, LedgerError> {
        Ok(self.db.query_row(
            "SELECT count(*) FROM manifests WHERE step_key = ?1",
            [step_key.to_string()],
            |r| r.get::<_, i64>(0),
        )? as usize)
    }

    pub fn manifests_for_key(&self, step_key: &Hash) -> Result<Vec<ManifestRow>, LedgerError> {
        let mut st = self.db.prepare(
            "SELECT id, run_id, created_at, json FROM manifests WHERE step_key = ?1 ORDER BY created_at DESC, rowid DESC",
        )?;
        let rows = st.query_map([step_key.to_string()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Vec<u8>>(3)?,
            ))
        })?;
        rows.map(|row| {
            let (id, run_id, created_at, json) = row?;
            let m: Manifest = serde_json::from_slice(&json)?;
            Ok(ManifestRow {
                id: id.parse().expect("ledger holds valid hashes"),
                run_id,
                created_at,
                entries: m.entries.len(),
            })
        })
        .collect()
    }

    pub fn manifest(&self, id: &Hash) -> Result<Option<Manifest>, LedgerError> {
        let json: Option<Vec<u8>> = self
            .db
            .query_row(
                "SELECT json FROM manifests WHERE id = ?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        json.map(|j| Ok(serde_json::from_slice(&j)?)).transpose()
    }

    /// The newest result recorded for this key and manifest.
    pub fn latest_result(
        &self,
        step_key: &Hash,
        manifest_id: &Hash,
    ) -> Result<Option<StepResult>, LedgerError> {
        let json: Option<Vec<u8>> = self
            .db
            .query_row(
                "SELECT json FROM results WHERE step_key = ?1 AND manifest_id = ?2 ORDER BY created_at DESC, rowid DESC LIMIT 1",
                params![step_key.to_string(), manifest_id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        json.map(|j| Ok(serde_json::from_slice(&j)?)).transpose()
    }

    pub fn results_for_run(&self, run_id: &str) -> Result<Vec<StepResult>, LedgerError> {
        let mut st = self
            .db
            .prepare("SELECT json FROM results WHERE run_id = ?1")?;
        let rows = st.query_map([run_id], |r| r.get::<_, Vec<u8>>(0))?;
        rows.map(|j| Ok(serde_json::from_slice(&j?)?)).collect()
    }

    /// Verifies the newest candidates against `tree` in order; the first
    /// match wins and its newest result is returned.
    pub fn find_match(&self, step_key: &Hash, tree: &Tree) -> Result<Lookup, LedgerError> {
        let cands = self.candidates(step_key, CANDIDATES)?;
        let total = self.manifest_count(step_key)?;
        let mut closest = None;
        for (i, m) in cands.iter().enumerate() {
            match matches(m, tree) {
                Ok(()) => {
                    let id = manifest_id(m)?;
                    if let Some(r) = self.latest_result(step_key, &id)? {
                        return Ok(Lookup {
                            hit: Some((m.clone(), r)),
                            candidates: total,
                            closest,
                        });
                    }
                }
                Err(mm) if i == 0 => closest = Some((manifest_id(m)?, *mm)),
                Err(_) => {}
            }
        }
        Ok(Lookup {
            hit: None,
            candidates: total,
            closest,
        })
    }
}

/// Now as `YYYY-MM-DDTHH:MM:SS.mmmZ` (UTC). Sorts lexically in time order.
pub fn now_rfc3339() -> String {
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before 1970");
    format_rfc3339(d.as_secs(), d.subsec_millis())
}

/// Seconds since the epoch as RFC 3339 UTC with milliseconds.
pub fn format_rfc3339(secs: u64, millis: u32) -> String {
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // Civil from days (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use sp_cas::Entry;
    use sp_manifest::{Access, Kind};

    fn tree(gomod: &str) -> Tree {
        Tree::from_leaves([
            Entry::file("go.mod", Hash::of(gomod.as_bytes()), 0o644),
            Entry::file("main.go", Hash::of(b"package main"), 0o644),
        ])
    }

    fn manifest(t: &Tree, key: Hash) -> Manifest {
        Manifest::from_log(
            key,
            vec!["/workspace".into()],
            t,
            [Access {
                path: "go.mod".into(),
                kind: Kind::File,
            }],
        )
        .unwrap()
    }

    fn result(key: Hash, m: Hash, run: &str, code: i32) -> StepResult {
        StepResult {
            v: 1,
            step_key: key,
            manifest_id: m,
            run_id: run.into(),
            recorded_at: now_rfc3339(),
            exit_code: code,
            stdout: Hash::of(b"out"),
            stderr: Hash::of(b""),
            outputs: vec![],
            duration_ms: 1000,
        }
    }

    #[test]
    fn rfc3339() {
        assert_eq!(format_rfc3339(0, 0), "1970-01-01T00:00:00.000Z");
        assert_eq!(format_rfc3339(1_731_575_523, 7), "2024-11-14T09:12:03.007Z");
        assert_eq!(format_rfc3339(951_782_400, 0), "2000-02-29T00:00:00.000Z");
    }

    #[test]
    fn migrates_from_empty_and_reopens() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("ledger.sqlite");
        let l = Ledger::open(&p).unwrap();
        assert_eq!(l.schema_version().unwrap(), 1);
        l.record_run("01RUN", Some("abc"), Some("box")).unwrap();
        drop(l);
        let l = Ledger::open(&p).unwrap();
        assert_eq!(l.schema_version().unwrap(), 1);
        assert_eq!(
            l.run("01RUN").unwrap().unwrap().commit_sha.as_deref(),
            Some("abc")
        );
    }

    #[test]
    fn candidates_newest_first_and_find_match() {
        let l = Ledger::open_in_memory().unwrap();
        let key = Hash::of(b"key");
        let (t1, t2, t3) = (tree("v1"), tree("v2"), tree("v3"));
        let (m1, m2) = (manifest(&t1, key), manifest(&t2, key));
        let id1 = l.put_manifest(&m1, "r1").unwrap();
        l.put_result(&result(key, id1, "r1", 0)).unwrap();
        let id2 = l.put_manifest(&m2, "r2").unwrap();
        l.put_result(&result(key, id2, "r2", 1)).unwrap();

        let c = l.candidates(&key, 50).unwrap();
        assert_eq!(c, [m2.clone(), m1.clone()]);

        // t1 matches the older candidate; the newest's mismatch is kept.
        let hit = l.find_match(&key, &t1).unwrap();
        let (m, r) = hit.hit.unwrap();
        assert_eq!((m, r.run_id.as_str(), r.exit_code), (m1.clone(), "r1", 0));
        let (closest_id, mm) = hit.closest.unwrap();
        assert_eq!((closest_id, mm.path.as_str()), (id2, "go.mod"));

        let miss = l.find_match(&key, &t3).unwrap();
        assert!(miss.hit.is_none());
        assert_eq!(miss.candidates, 2);
        assert_eq!(miss.closest.unwrap().0, id2);

        let none = l.find_match(&Hash::of(b"other key"), &t1).unwrap();
        assert!(none.hit.is_none() && none.closest.is_none() && none.candidates == 0);
    }

    #[test]
    fn newest_result_wins_and_rerecording_refreshes() {
        let l = Ledger::open_in_memory().unwrap();
        let key = Hash::of(b"key");
        let t = tree("v1");
        let m = manifest(&t, key);
        let id = l.put_manifest(&m, "r1").unwrap();
        l.put_result(&result(key, id, "r1", 0)).unwrap();
        l.put_manifest(&manifest(&tree("v2"), key), "r2").unwrap();
        assert_eq!(l.put_manifest(&m, "r3").unwrap(), id);
        l.put_result(&result(key, id, "r3", 0)).unwrap();
        assert_eq!(l.candidates(&key, 1).unwrap(), [m]);
        assert_eq!(l.latest_result(&key, &id).unwrap().unwrap().run_id, "r3");
        assert_eq!(l.manifests_for_key(&key).unwrap().len(), 2);
    }
}
