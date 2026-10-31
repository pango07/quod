//! SQLite ledger: manifests and results by step key, runs, the blob index,
//! and the trust state: per-key trust rows, per-manifest proof streaks and
//! the audit log. Single writer, WAL mode. Blobs live in the CAS; the
//! `blobs` table is an index, not the store.
//!
//! The ledger stores trust facts and knows nothing about how they are
//! decided; that is `sp-trust`.

use std::cell::Cell;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};
use sp_cas::{Hash, Tree};
use sp_manifest::{Manifest, Mismatch, StepResult, manifest_id, matches};

/// How many candidate manifests a lookup verifies, newest first.
pub const CANDIDATES: usize = 50;

/// Schema version this binary writes: one per migration.
pub const SCHEMA_VERSION: usize = MIGRATIONS.len();

const MIGRATIONS: &[&str] = &[
    // v1: the Schemas section.
    "CREATE TABLE runs      (id TEXT PRIMARY KEY, started_at TEXT NOT NULL, commit_sha TEXT, host TEXT);
     CREATE TABLE manifests (id TEXT PRIMARY KEY, step_key TEXT NOT NULL, run_id TEXT NOT NULL, created_at TEXT NOT NULL, json BLOB NOT NULL);
     CREATE INDEX manifests_by_key ON manifests (step_key, created_at DESC);
     CREATE TABLE results   (id TEXT PRIMARY KEY, step_key TEXT NOT NULL, manifest_id TEXT NOT NULL, run_id TEXT NOT NULL, created_at TEXT NOT NULL, json BLOB NOT NULL);
     CREATE INDEX results_by_key ON results (step_key, manifest_id, created_at DESC);
     CREATE TABLE blobs     (hash TEXT PRIMARY KEY, size INTEGER NOT NULL, path TEXT NOT NULL);",
    // v2: which observer transport served each executed run (`fuse-uring`,
    // `fuse`) and the full observer description, so results a transport
    // produced can be found and purged. NULL: a replay (nothing observed),
    // or a run recorded before v2 (transport unknown).
    "ALTER TABLE runs ADD COLUMN transport TEXT;
     ALTER TABLE runs ADD COLUMN observer TEXT;
     CREATE INDEX runs_by_transport ON runs (transport);",
    // v3: the canonical JSON behind each step key, so a result's environment
    // pin (rootfs digest or host fingerprint, platform) can be read back.
    "CREATE TABLE step_keys (step_key TEXT PRIMARY KEY, json BLOB NOT NULL);",
    // v4: trust. Runs carry what `status` needs (which repo, which key, how
    // it ended, what was saved); `trust` is one row per step key; `proofs`
    // is one row per (key, manifest) with the streak of consecutive
    // executions that produced the same result; `audits` is every
    // re-execution of a manifest that already had a result, with whether it
    // diverged.
    "ALTER TABLE runs ADD COLUMN step_key TEXT;
     ALTER TABLE runs ADD COLUMN repo TEXT;
     ALTER TABLE runs ADD COLUMN repo_path TEXT;
     ALTER TABLE runs ADD COLUMN mode TEXT;
     ALTER TABLE runs ADD COLUMN outcome TEXT;
     ALTER TABLE runs ADD COLUMN manifest_id TEXT;
     ALTER TABLE runs ADD COLUMN exit_code INTEGER;
     ALTER TABLE runs ADD COLUMN wall_ms INTEGER;
     ALTER TABLE runs ADD COLUMN saved_ms INTEGER;
     CREATE INDEX runs_by_repo ON runs (repo, started_at DESC);
     CREATE TABLE trust  (step_key TEXT PRIMARY KEY, cmd TEXT, state TEXT NOT NULL, eligible INTEGER NOT NULL DEFAULT 0, replays INTEGER NOT NULL DEFAULT 0, audits INTEGER NOT NULL DEFAULT 0, divergences INTEGER NOT NULL DEFAULT 0, quarantined_at TEXT, quarantine_reason TEXT, updated_at TEXT NOT NULL);
     CREATE TABLE proofs (step_key TEXT NOT NULL, manifest_id TEXT NOT NULL, streak INTEGER NOT NULL, fingerprint TEXT NOT NULL, last_run TEXT NOT NULL, updated_at TEXT NOT NULL, PRIMARY KEY (step_key, manifest_id));
     CREATE TABLE audits (id INTEGER PRIMARY KEY AUTOINCREMENT, step_key TEXT NOT NULL, manifest_id TEXT NOT NULL, at TEXT NOT NULL, phase TEXT NOT NULL, diverged INTEGER NOT NULL, expected_run TEXT, actual_run TEXT, detail TEXT);
     CREATE INDEX audits_by_key ON audits (step_key, id DESC);",
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
    #[error("ledger holds an invalid value: {0}")]
    Corrupt(String),
}

pub struct Ledger {
    db: Connection,
    /// Inside `atomically` (which nests by joining the outer transaction).
    in_tx: Cell<bool>,
}

/// How long a writer waits for another process's transaction to finish.
/// Several `stillpass run`s (an agent's parallel tool calls) may share one
/// ledger; each trust update is a short read-modify-write.
const BUSY_TIMEOUT: Duration = Duration::from_secs(60);

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

/// How an executed run was observed, stored with the run.
#[derive(Debug, Clone, Copy, Default)]
pub struct Observed<'a> {
    pub transport: Option<&'a str>,
    pub observer: Option<&'a str>,
}

/// One run. The v1 columns are always present; the rest were added later
/// and are empty for runs recorded before them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunRow {
    pub id: String,
    pub started_at: String,
    pub commit_sha: Option<String>,
    pub host: Option<String>,
    /// The observer transport that served the run (`fuse-uring`, `fuse`);
    /// `None` for a replay or a run recorded before schema v2.
    pub transport: Option<String>,
    /// The full observer description, e.g. `fuse-uring+passthrough+sealed`.
    pub observer: Option<String>,
    pub step_key: Option<Hash>,
    /// The repo's identity (its root commit, else a hash of its path).
    pub repo: Option<String>,
    pub repo_path: Option<String>,
    pub mode: Option<String>,
    /// How the run ended: `replayed`, `would-replay`, `miss`, `proving`,
    /// `unprovable`, `audit`, `diverged`, `timeout`, `killed`, `net`.
    pub outcome: Option<String>,
    pub manifest_id: Option<Hash>,
    pub exit_code: Option<i32>,
    pub wall_ms: Option<u64>,
    /// Time a replay saved, or (observe mode) would have saved.
    pub saved_ms: Option<u64>,
}

/// Where a step key stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustState {
    /// No manifest of the key has been reproduced enough times yet.
    Observing,
    /// Some manifest of the key has; proven manifests may replay.
    Proven,
    /// A divergence was seen; nothing replays until a manifest re-proves.
    Quarantined,
}

impl TrustState {
    pub fn as_str(self) -> &'static str {
        match self {
            TrustState::Observing => "observing",
            TrustState::Proven => "proven",
            TrustState::Quarantined => "quarantined",
        }
    }

    fn parse(s: &str) -> Result<TrustState, LedgerError> {
        match s {
            "observing" => Ok(TrustState::Observing),
            "proven" => Ok(TrustState::Proven),
            "quarantined" => Ok(TrustState::Quarantined),
            other => Err(LedgerError::Corrupt(format!("trust state {other:?}"))),
        }
    }
}

/// The trust row of one step key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustRow {
    pub step_key: Hash,
    /// The step's script, for `status`.
    pub cmd: Option<String>,
    pub state: TrustState,
    /// Would-be hits so far: replays served plus audited or observed hits.
    pub eligible: u64,
    /// Replays served.
    pub replays: u64,
    /// Re-executions compared against a recorded result.
    pub audits: u64,
    pub divergences: u64,
    pub quarantined_at: Option<String>,
    pub quarantine_reason: Option<String>,
    pub updated_at: String,
}

impl TrustRow {
    pub fn new(step_key: Hash, cmd: Option<&str>) -> TrustRow {
        TrustRow {
            step_key,
            cmd: cmd.map(String::from),
            state: TrustState::Observing,
            eligible: 0,
            replays: 0,
            audits: 0,
            divergences: 0,
            quarantined_at: None,
            quarantine_reason: None,
            updated_at: now_rfc3339(),
        }
    }
}

/// The proof streak of one manifest of one key: how many consecutive
/// executions produced the result `fingerprint` names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofRow {
    pub step_key: Hash,
    pub manifest_id: Hash,
    pub streak: u32,
    pub fingerprint: Hash,
    pub last_run: String,
    pub updated_at: String,
}

/// One re-execution of a manifest that already had a result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRow {
    pub id: i64,
    pub step_key: Hash,
    pub manifest_id: Hash,
    pub at: String,
    /// `proof` (not yet replayable), `audit` (a sampled replay-mode hit) or
    /// `observe` (observe mode, would have replayed).
    pub phase: String,
    pub diverged: bool,
    pub expected_run: Option<String>,
    pub actual_run: Option<String>,
    pub detail: Option<String>,
}

/// What `record_run_facts` stores about a run.
#[derive(Debug, Clone, Default)]
pub struct RunFacts<'a> {
    pub id: &'a str,
    pub commit_sha: Option<&'a str>,
    pub host: Option<&'a str>,
    /// How it was observed (an executed run; `None` for a replay).
    pub observed: Observed<'a>,
    pub step_key: Option<&'a Hash>,
    pub repo: Option<&'a str>,
    pub repo_path: Option<&'a str>,
    pub mode: Option<&'a str>,
    pub outcome: Option<&'a str>,
    pub manifest_id: Option<&'a Hash>,
    pub exit_code: Option<i32>,
    pub wall_ms: Option<u64>,
    pub saved_ms: Option<u64>,
}

impl Ledger {
    pub fn open(path: &Path) -> Result<Ledger, LedgerError> {
        let db = Connection::open(path)?;
        db.busy_timeout(BUSY_TIMEOUT)?;
        db.pragma_update(None, "journal_mode", "WAL")?;
        db.pragma_update(None, "synchronous", "NORMAL")?;
        let mut l = Ledger::wrap(db);
        l.migrate()?;
        Ok(l)
    }

    pub fn open_in_memory() -> Result<Ledger, LedgerError> {
        let mut l = Ledger::wrap(Connection::open_in_memory()?);
        l.migrate()?;
        Ok(l)
    }

    fn wrap(db: Connection) -> Ledger {
        Ledger {
            db,
            in_tx: Cell::new(false),
        }
    }

    /// Runs `f` in one write transaction (BEGIN IMMEDIATE: the write lock
    /// is taken up front, so a read-modify-write of a trust row can never
    /// interleave with another process's), committed if `f` succeeds and
    /// rolled back if it fails. A call inside `f` joins the transaction.
    pub fn atomically<T, E: From<LedgerError>>(
        &self,
        f: impl FnOnce(&Ledger) -> Result<T, E>,
    ) -> Result<T, E> {
        if self.in_tx.get() {
            return f(self);
        }
        self.db
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(LedgerError::from)?;
        self.in_tx.set(true);
        let out = f(self);
        self.in_tx.set(false);
        match out {
            Ok(v) => {
                self.db.execute_batch("COMMIT").map_err(LedgerError::from)?;
                Ok(v)
            }
            Err(e) => {
                let _ = self.db.execute_batch("ROLLBACK");
                Err(e)
            }
        }
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

    /// The schema as SQLite stores it, one statement per line, sorted: the
    /// golden `fixtures/ledger_schema.sql` is this.
    pub fn schema_sql(&self) -> Result<String, LedgerError> {
        let mut st = self.db.prepare(
            "SELECT sql FROM sqlite_master WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' ORDER BY type DESC, name",
        )?;
        let rows: Result<Vec<String>, _> = st.query_map([], |r| r.get::<_, String>(0))?.collect();
        let mut out = String::new();
        for s in rows? {
            out.push_str(&s.split_whitespace().collect::<Vec<_>>().join(" "));
            out.push_str(";\n");
        }
        Ok(out)
    }

    /// Records a run with the facts every run has (see `record_run_facts`).
    pub fn record_run(
        &self,
        id: &str,
        commit_sha: Option<&str>,
        host: Option<&str>,
        observed: Observed,
    ) -> Result<(), LedgerError> {
        self.record_run_facts(&RunFacts {
            id,
            commit_sha,
            host,
            observed,
            ..RunFacts::default()
        })
    }

    pub fn record_run_facts(&self, f: &RunFacts) -> Result<(), LedgerError> {
        insert_run(&self.db, f)
    }

    /// Records an executed run: its run row (with the transport that
    /// observed it), its manifest and its result, in one transaction. A
    /// crash or an error part-way leaves none of them, so a result is never
    /// visible without the run row `results_by_transport` and
    /// `purge_transport` find it through. Returns the manifest id.
    pub fn record_execution(
        &self,
        run_id: &str,
        commit_sha: Option<&str>,
        host: Option<&str>,
        observed: Observed,
        m: &Manifest,
        r: &StepResult,
    ) -> Result<Hash, LedgerError> {
        self.record_execution_facts(
            &RunFacts {
                id: run_id,
                commit_sha,
                host,
                observed,
                ..RunFacts::default()
            },
            m,
            r,
        )
    }

    /// `record_execution` with every fact of the run row. It is one
    /// transaction (`atomically`), so called inside a caller's
    /// `atomically` it joins that one: the trust layer's verdict, the run,
    /// the manifest and the result commit together or not at all.
    pub fn record_execution_facts(
        &self,
        run: &RunFacts,
        m: &Manifest,
        r: &StepResult,
    ) -> Result<Hash, LedgerError> {
        self.atomically(|l| {
            insert_run(&l.db, run)?;
            let id = insert_manifest(&l.db, m, run.id)?;
            insert_result(&l.db, r)?;
            Ok(id)
        })
    }

    /// Rows per table, counted directly.
    pub fn row_counts(&self) -> Result<RowCounts, LedgerError> {
        let n = |t: &str| -> Result<usize, LedgerError> {
            Ok(self
                .db
                .query_row(&format!("SELECT count(*) FROM {t}"), [], |r| {
                    r.get::<_, i64>(0)
                })? as usize)
        };
        Ok(RowCounts {
            runs: n("runs")?,
            manifests: n("manifests")?,
            results: n("results")?,
        })
    }

    const RUN_COLUMNS: &'static str = "id, started_at, commit_sha, host, transport, observer, step_key, repo, repo_path, mode, outcome, manifest_id, exit_code, wall_ms, saved_ms";

    fn run_row(r: &rusqlite::Row) -> rusqlite::Result<RunRow> {
        let hash = |v: Option<String>| v.and_then(|s| s.parse().ok());
        Ok(RunRow {
            id: r.get(0)?,
            started_at: r.get(1)?,
            commit_sha: r.get(2)?,
            host: r.get(3)?,
            transport: r.get(4)?,
            observer: r.get(5)?,
            step_key: hash(r.get(6)?),
            repo: r.get(7)?,
            repo_path: r.get(8)?,
            mode: r.get(9)?,
            outcome: r.get(10)?,
            manifest_id: hash(r.get(11)?),
            exit_code: r.get(12)?,
            wall_ms: r.get::<_, Option<i64>>(13)?.map(|v| v as u64),
            saved_ms: r.get::<_, Option<i64>>(14)?.map(|v| v as u64),
        })
    }

    pub fn run(&self, id: &str) -> Result<Option<RunRow>, LedgerError> {
        Ok(self
            .db
            .query_row(
                &format!("SELECT {} FROM runs WHERE id = ?1", Self::RUN_COLUMNS),
                [id],
                Self::run_row,
            )
            .optional()?)
    }

    /// Every run of `repo` (its identity, see `RunRow::repo`), or every run
    /// when `None`, oldest first.
    pub fn runs(&self, repo: Option<&str>) -> Result<Vec<RunRow>, LedgerError> {
        let sql = format!(
            "SELECT {} FROM runs WHERE (?1 IS NULL OR repo = ?1) ORDER BY started_at, rowid",
            Self::RUN_COLUMNS
        );
        let mut st = self.db.prepare(&sql)?;
        let rows = st.query_map(params![repo], Self::run_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Every result recorded by a run that `transport` observed (`None`:
    /// results whose run has no transport, i.e. recorded before schema v2).
    pub fn results_by_transport(
        &self,
        transport: Option<&str>,
    ) -> Result<Vec<StepResult>, LedgerError> {
        let mut st = self.db.prepare(
            "SELECT r.json FROM results r JOIN runs u ON u.id = r.run_id WHERE u.transport IS ?1 ORDER BY r.created_at, r.rowid",
        )?;
        let rows = st.query_map([transport], |r| r.get::<_, Vec<u8>>(0))?;
        rows.map(|j| Ok(serde_json::from_slice(&j?)?)).collect()
    }

    /// Deletes every result recorded by a run that `transport` observed and
    /// returns how many. Their manifests stay but can no longer produce a
    /// hit: a lookup replays only a manifest that still has a result.
    pub fn purge_transport(&self, transport: Option<&str>) -> Result<usize, LedgerError> {
        Ok(self.db.execute(
            "DELETE FROM results WHERE run_id IN (SELECT id FROM runs WHERE transport IS ?1)",
            [transport],
        )?)
    }

    /// Stores a manifest and returns its id. Re-recording an identical
    /// manifest refreshes its position as newest.
    pub fn put_manifest(&self, m: &Manifest, run_id: &str) -> Result<Hash, LedgerError> {
        insert_manifest(&self.db, m, run_id)
    }

    pub fn put_result(&self, r: &StepResult) -> Result<(), LedgerError> {
        insert_result(&self.db, r)
    }

    /// Records the canonical JSON a step key was computed from (idempotent).
    pub fn put_step_key(&self, step_key: &Hash, json: &str) -> Result<(), LedgerError> {
        self.db.execute(
            "INSERT OR IGNORE INTO step_keys (step_key, json) VALUES (?1, ?2)",
            params![step_key.to_string(), json.as_bytes()],
        )?;
        Ok(())
    }

    /// The canonical JSON behind a step key, if this ledger recorded it.
    pub fn step_key_json(&self, step_key: &Hash) -> Result<Option<String>, LedgerError> {
        let json: Option<Vec<u8>> = self
            .db
            .query_row(
                "SELECT json FROM step_keys WHERE step_key = ?1",
                [step_key.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        Ok(json.map(|j| String::from_utf8_lossy(&j).into_owned()))
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
        let closest = match cands.first().map(|m| matches(m, tree)) {
            Some(Err(mm)) => Some((manifest_id(&cands[0])?, *mm)),
            _ => None,
        };
        let mut from = 0;
        while let Some(i) = first_match(&cands[from..], tree).map(|i| i + from) {
            if let Some(r) = self.latest_result(step_key, &manifest_id(&cands[i])?)? {
                return Ok(Lookup {
                    hit: Some((cands[i].clone(), r)),
                    candidates: total,
                    closest,
                });
            }
            from = i + 1;
        }
        Ok(Lookup {
            hit: None,
            candidates: total,
            closest,
        })
    }

    // ----- trust (v3) -----

    pub fn trust(&self, step_key: &Hash) -> Result<Option<TrustRow>, LedgerError> {
        let row = self
            .db
            .query_row(
                "SELECT step_key, cmd, state, eligible, replays, audits, divergences, quarantined_at, quarantine_reason, updated_at FROM trust WHERE step_key = ?1",
                [step_key.to_string()],
                Self::trust_row,
            )
            .optional()?;
        row.map(Self::finish_trust).transpose()
    }

    /// Every trust row, most recently updated first.
    pub fn trust_rows(&self) -> Result<Vec<TrustRow>, LedgerError> {
        let mut st = self.db.prepare(
            "SELECT step_key, cmd, state, eligible, replays, audits, divergences, quarantined_at, quarantine_reason, updated_at FROM trust ORDER BY updated_at DESC",
        )?;
        let rows: Vec<_> = st
            .query_map([], Self::trust_row)?
            .collect::<Result<_, _>>()?;
        rows.into_iter().map(Self::finish_trust).collect()
    }

    fn trust_row(r: &rusqlite::Row) -> rusqlite::Result<RawTrust> {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
            r.get(6)?,
            r.get(7)?,
            r.get(8)?,
            r.get(9)?,
        ))
    }

    fn finish_trust(raw: RawTrust) -> Result<TrustRow, LedgerError> {
        let (key, cmd, state, eligible, replays, audits, divergences, qa, qr, updated_at) = raw;
        Ok(TrustRow {
            step_key: parse_hash(&key)?,
            cmd,
            state: TrustState::parse(&state)?,
            eligible: eligible as u64,
            replays: replays as u64,
            audits: audits as u64,
            divergences: divergences as u64,
            quarantined_at: qa,
            quarantine_reason: qr,
            updated_at,
        })
    }

    pub fn put_trust(&self, t: &TrustRow) -> Result<(), LedgerError> {
        self.db.execute(
            "INSERT OR REPLACE INTO trust (step_key, cmd, state, eligible, replays, audits, divergences, quarantined_at, quarantine_reason, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                t.step_key.to_string(),
                t.cmd,
                t.state.as_str(),
                t.eligible as i64,
                t.replays as i64,
                t.audits as i64,
                t.divergences as i64,
                t.quarantined_at,
                t.quarantine_reason,
                now_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn proof(
        &self,
        step_key: &Hash,
        manifest_id: &Hash,
    ) -> Result<Option<ProofRow>, LedgerError> {
        let row = self
            .db
            .query_row(
                "SELECT step_key, manifest_id, streak, fingerprint, last_run, updated_at FROM proofs WHERE step_key = ?1 AND manifest_id = ?2",
                params![step_key.to_string(), manifest_id.to_string()],
                Self::proof_row,
            )
            .optional()?;
        row.map(Self::finish_proof).transpose()
    }

    /// Every proof row of a key, longest streak first.
    pub fn proofs_for_key(&self, step_key: &Hash) -> Result<Vec<ProofRow>, LedgerError> {
        let mut st = self.db.prepare(
            "SELECT step_key, manifest_id, streak, fingerprint, last_run, updated_at FROM proofs WHERE step_key = ?1 ORDER BY streak DESC, updated_at DESC",
        )?;
        let rows: Vec<_> = st
            .query_map([step_key.to_string()], Self::proof_row)?
            .collect::<Result<_, _>>()?;
        rows.into_iter().map(Self::finish_proof).collect()
    }

    fn proof_row(r: &rusqlite::Row) -> rusqlite::Result<RawProof> {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
        ))
    }

    fn finish_proof(raw: RawProof) -> Result<ProofRow, LedgerError> {
        let (key, mid, streak, fp, last_run, updated_at) = raw;
        Ok(ProofRow {
            step_key: parse_hash(&key)?,
            manifest_id: parse_hash(&mid)?,
            streak: streak.max(0) as u32,
            fingerprint: parse_hash(&fp)?,
            last_run,
            updated_at,
        })
    }

    pub fn put_proof(&self, p: &ProofRow) -> Result<(), LedgerError> {
        self.db.execute(
            "INSERT OR REPLACE INTO proofs (step_key, manifest_id, streak, fingerprint, last_run, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                p.step_key.to_string(),
                p.manifest_id.to_string(),
                p.streak as i64,
                p.fingerprint.to_string(),
                p.last_run,
                now_rfc3339(),
            ],
        )?;
        Ok(())
    }

    /// Resets every proof streak of a key to zero (a quarantine): each
    /// manifest keeps its last fingerprint to compare the next execution
    /// against, but must start proving again.
    pub fn reset_proofs(&self, step_key: &Hash) -> Result<usize, LedgerError> {
        Ok(self.db.execute(
            "UPDATE proofs SET streak = 0, updated_at = ?2 WHERE step_key = ?1",
            params![step_key.to_string(), now_rfc3339()],
        )?)
    }

    /// Resets one manifest's proof streak to zero (its last fingerprint and
    /// run stay, to compare the next execution against).
    pub fn reset_proof(&self, step_key: &Hash, manifest_id: &Hash) -> Result<usize, LedgerError> {
        Ok(self.db.execute(
            "UPDATE proofs SET streak = 0, updated_at = ?3 WHERE step_key = ?1 AND manifest_id = ?2",
            params![step_key.to_string(), manifest_id.to_string(), now_rfc3339()],
        )?)
    }

    /// How many of a key's newest `window` audits diverged.
    pub fn recent_divergences(&self, step_key: &Hash, window: usize) -> Result<u64, LedgerError> {
        Ok(self.db.query_row(
            "SELECT count(*) FROM (SELECT diverged FROM audits WHERE step_key = ?1 ORDER BY id DESC LIMIT ?2) WHERE diverged != 0",
            params![step_key.to_string(), window as i64],
            |r| r.get::<_, i64>(0),
        )? as u64)
    }

    /// Appends an audit (the row's `id` and `at` are assigned here).
    pub fn record_audit(&self, a: &AuditRow) -> Result<i64, LedgerError> {
        self.db.execute(
            "INSERT INTO audits (step_key, manifest_id, at, phase, diverged, expected_run, actual_run, detail) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                a.step_key.to_string(),
                a.manifest_id.to_string(),
                now_rfc3339(),
                a.phase,
                a.diverged as i64,
                a.expected_run,
                a.actual_run,
                a.detail,
            ],
        )?;
        Ok(self.db.last_insert_rowid())
    }

    /// The newest `limit` audits, for one key or (None) for all keys.
    pub fn audits(
        &self,
        step_key: Option<&Hash>,
        limit: usize,
    ) -> Result<Vec<AuditRow>, LedgerError> {
        let mut st = self.db.prepare(
            "SELECT id, step_key, manifest_id, at, phase, diverged, expected_run, actual_run, detail FROM audits WHERE (?1 IS NULL OR step_key = ?1) ORDER BY id DESC LIMIT ?2",
        )?;
        let rows: Vec<RawAudit> = st
            .query_map(
                params![step_key.map(|h| h.to_string()), limit as i64],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                        r.get(8)?,
                    ))
                },
            )?
            .collect::<Result<_, _>>()?;
        rows.into_iter()
            .map(|(id, key, mid, at, phase, diverged, e, a, d)| {
                Ok(AuditRow {
                    id,
                    step_key: parse_hash(&key)?,
                    manifest_id: parse_hash(&mid)?,
                    at,
                    phase,
                    diverged: diverged != 0,
                    expected_run: e,
                    actual_run: a,
                    detail: d,
                })
            })
            .collect()
    }
}

/// A trust row as SQLite hands it over, before parsing.
type RawTrust = (
    String,
    Option<String>,
    String,
    i64,
    i64,
    i64,
    i64,
    Option<String>,
    Option<String>,
    String,
);

/// A proof row as SQLite hands it over, before parsing.
type RawProof = (String, String, i64, String, String, String);

/// An audit row as SQLite hands it over, before parsing.
type RawAudit = (
    i64,
    String,
    String,
    String,
    String,
    i64,
    Option<String>,
    Option<String>,
    Option<String>,
);

fn parse_hash(s: &str) -> Result<Hash, LedgerError> {
    s.parse()
        .map_err(|_| LedgerError::Corrupt(format!("hash {s:?}")))
}

/// Rows per ledger table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowCounts {
    pub runs: usize,
    pub manifests: usize,
    pub results: usize,
}

fn insert_run(db: &Connection, f: &RunFacts) -> Result<(), LedgerError> {
    db.execute(
        "INSERT INTO runs (id, started_at, commit_sha, host, transport, observer, step_key, repo, repo_path, mode, outcome, manifest_id, exit_code, wall_ms, saved_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        params![
            f.id,
            now_rfc3339(),
            f.commit_sha,
            f.host,
            f.observed.transport,
            f.observed.observer,
            f.step_key.map(|h| h.to_string()),
            f.repo,
            f.repo_path,
            f.mode,
            f.outcome,
            f.manifest_id.map(|h| h.to_string()),
            f.exit_code,
            f.wall_ms.map(|v| v as i64),
            f.saved_ms.map(|v| v as i64),
        ],
    )?;
    Ok(())
}

fn insert_manifest(db: &Connection, m: &Manifest, run_id: &str) -> Result<Hash, LedgerError> {
    let id = manifest_id(m)?;
    db.execute(
        // REPLACE gives the row a new rowid, which breaks created_at ties.
        "INSERT OR REPLACE INTO manifests (id, step_key, run_id, created_at, json) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id.to_string(), m.step_key.to_string(), run_id, now_rfc3339(), m.canonical_json()?.into_bytes()],
    )?;
    Ok(id)
}

fn insert_result(db: &Connection, r: &StepResult) -> Result<(), LedgerError> {
    let json = sp_cas::canonical_json(r).map_err(sp_manifest::ManifestError::from)?;
    db.execute(
        "INSERT INTO results (id, step_key, manifest_id, run_id, created_at, json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![r.run_id, r.step_key.to_string(), r.manifest_id.to_string(), r.run_id, now_rfc3339(), json.into_bytes()],
    )?;
    Ok(())
}

/// The index of the first candidate, in order, that matches `tree`. This is
/// the whole replay decision; `find_match` adds storage around it.
pub fn first_match(cands: &[Manifest], tree: &Tree) -> Option<usize> {
    cands.iter().position(|m| matches(m, tree).is_ok())
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
            root_outputs: vec![],
            duration_ms: 1000,
            commands: vec![],
        }
    }

    /// A ledger as the v1 binary left it: the v1 schema with rows in it.
    fn v1_ledger() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(include_str!("../../../fixtures/ledger_v1.sql"))
            .unwrap();
        db.pragma_update(None, "user_version", 1).unwrap();
        db
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
        assert_eq!(l.schema_version().unwrap(), MIGRATIONS.len());
        l.record_run("01RUN", Some("abc"), Some("box"), Observed::default())
            .unwrap();
        let key = Hash::of(b"key");
        l.put_step_key(&key, r#"{"image":{"kind":"rootfs"}}"#)
            .unwrap();
        l.put_step_key(&key, "other").unwrap();
        drop(l);
        let l = Ledger::open(&p).unwrap();
        assert_eq!(l.schema_version().unwrap(), MIGRATIONS.len());
        assert_eq!(
            l.run("01RUN").unwrap().unwrap().commit_sha.as_deref(),
            Some("abc")
        );
        assert_eq!(
            l.step_key_json(&key).unwrap().as_deref(),
            Some(r#"{"image":{"kind":"rootfs"}}"#),
            "the first JSON recorded for a key stays"
        );
        assert_eq!(l.step_key_json(&Hash::of(b"none")).unwrap(), None);
    }

    /// A v2 ledger (transports, no key documents) migrates to v3 in place:
    /// its runs keep their transports and key documents start empty.
    #[test]
    fn a_v2_ledger_migrates_to_v3() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("ledger.sqlite");
        {
            let db = Connection::open(&p).unwrap();
            db.execute_batch(MIGRATIONS[0]).unwrap();
            db.execute_batch(MIGRATIONS[1]).unwrap();
            db.pragma_update(None, "user_version", 2).unwrap();
            db.execute(
                "INSERT INTO runs (id, started_at, transport) VALUES ('01OLD', 'x', 'fuse')",
                [],
            )
            .unwrap();
        }
        let l = Ledger::open(&p).unwrap();
        assert_eq!(l.schema_version().unwrap(), SCHEMA_VERSION);
        assert_eq!(
            l.run("01OLD").unwrap().unwrap().transport.as_deref(),
            Some("fuse")
        );
        assert_eq!(l.step_key_json(&Hash::of(b"k")).unwrap(), None);
        l.put_step_key(&Hash::of(b"k"), "{}").unwrap();
        assert_eq!(
            l.step_key_json(&Hash::of(b"k")).unwrap().as_deref(),
            Some("{}")
        );
    }

    /// A v1 ledger (before the transport was stored) migrates in place: its
    /// runs keep everything and have no transport, which is how their
    /// results are found (and purged) as "unknown transport".
    #[test]
    fn migrates_v1_and_finds_results_by_transport() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("ledger.sqlite");
        {
            let db = Connection::open(&p).unwrap();
            db.execute_batch(MIGRATIONS[0]).unwrap();
            db.pragma_update(None, "user_version", 1).unwrap();
            db.execute(
                "INSERT INTO runs (id, started_at, commit_sha, host) VALUES ('r0', 't', 'c0', 'h')",
                [],
            )
            .unwrap();
        }
        let l = Ledger::open(&p).unwrap();
        assert_eq!(l.schema_version().unwrap(), SCHEMA_VERSION);
        let old = l.run("r0").unwrap().unwrap();
        assert_eq!(
            (old.commit_sha.as_deref(), old.transport, old.observer),
            (Some("c0"), None, None)
        );

        let key = Hash::of(b"key");
        let t = tree("v1");
        let id = l.put_manifest(&manifest(&t, key), "r0").unwrap();
        l.put_result(&result(key, id, "r0", 0)).unwrap();
        for (run, transport, observer) in [
            ("r1", "fuse-uring", "fuse-uring+passthrough+sealed"),
            ("r2", "fuse", "fuse+uring-fallback+passthrough+sealed"),
            ("r3", "fuse-uring", "fuse-uring+passthrough"),
        ] {
            l.record_run(
                run,
                None,
                None,
                Observed {
                    transport: Some(transport),
                    observer: Some(observer),
                },
            )
            .unwrap();
            l.put_result(&result(key, id, run, 0)).unwrap();
        }
        l.record_run("r4", None, None, Observed::default()).unwrap(); // a replay
        assert_eq!(
            l.run("r2").unwrap().unwrap().observer.as_deref(),
            Some("fuse+uring-fallback+passthrough+sealed")
        );
        let runs = |tr: Option<&str>| -> Vec<String> {
            l.results_by_transport(tr)
                .unwrap()
                .into_iter()
                .map(|r| r.run_id)
                .collect()
        };
        assert_eq!(runs(Some("fuse-uring")), ["r1", "r3"]);
        assert_eq!(runs(Some("fuse")), ["r2"]);
        assert_eq!(runs(None), ["r0"]);

        // Purging the ring's results leaves a hit only from the others.
        assert_eq!(l.purge_transport(Some("fuse-uring")).unwrap(), 2);
        assert!(runs(Some("fuse-uring")).is_empty());
        assert_eq!(l.latest_result(&key, &id).unwrap().unwrap().run_id, "r2");
        assert_eq!(l.purge_transport(Some("fuse")).unwrap(), 1);
        assert_eq!(l.purge_transport(None).unwrap(), 1);
        assert!(l.find_match(&key, &t).unwrap().hit.is_none());
        drop(l);
        // Reopening a migrated ledger runs nothing again.
        assert_eq!(
            Ledger::open(&p).unwrap().schema_version().unwrap(),
            SCHEMA_VERSION
        );
    }

    /// An execution is recorded whole or not at all: when its result cannot
    /// be written, neither its run row nor its manifest is left behind.
    #[test]
    fn record_execution_is_all_or_nothing() {
        let l = Ledger::open_in_memory().unwrap();
        let key = Hash::of(b"key");
        let t = tree("v1");
        let m = manifest(&t, key);
        let mid = manifest_id(&m).unwrap();
        let ring = Observed {
            transport: Some("fuse-uring"),
            observer: Some("fuse-uring+nice-10"),
        };
        assert_eq!(
            l.record_execution("r1", Some("c"), None, ring, &m, &result(key, mid, "r1", 0))
                .unwrap(),
            mid
        );
        assert_eq!(
            l.row_counts().unwrap(),
            RowCounts {
                runs: 1,
                manifests: 1,
                results: 1
            }
        );
        assert_eq!(l.results_by_transport(Some("fuse-uring")).unwrap().len(), 1);
        // A result row "r2" already exists (written before v2, say), so the
        // insert of r2's result fails after its run and manifest went in.
        l.put_result(&result(key, mid, "r2", 0)).unwrap();
        let m2 = manifest(&tree("v2"), key);
        let mid2 = manifest_id(&m2).unwrap();
        assert!(
            l.record_execution("r2", None, None, ring, &m2, &result(key, mid2, "r2", 0))
                .is_err()
        );
        assert!(l.run("r2").unwrap().is_none(), "the run row stayed");
        assert!(l.manifest(&mid2).unwrap().is_none(), "the manifest stayed");
        assert_eq!(l.row_counts().unwrap().results, 2);
        // A run id that exists already fails before anything is written.
        assert!(
            l.record_execution("r1", None, None, ring, &m2, &result(key, mid2, "r3", 0))
                .is_err()
        );
        assert!(l.manifest(&mid2).unwrap().is_none());
        assert_eq!(l.row_counts().unwrap().results, 2);
    }

    /// v1 -> current: the rows a v1 binary wrote survive and read back with
    /// the new columns empty; the schema equals the golden, and equals what
    /// a fresh ledger gets (so migrated and new ledgers never drift apart).
    #[test]
    fn migrates_a_v1_ledger_keeping_its_rows() {
        let mut l = Ledger::wrap(v1_ledger());
        assert_eq!(l.schema_version().unwrap(), 1);
        l.migrate().unwrap();
        assert_eq!(l.schema_version().unwrap(), SCHEMA_VERSION);

        let golden = include_str!("../../../fixtures/ledger_schema.sql");
        assert_eq!(l.schema_sql().unwrap(), golden, "schema golden");
        let fresh = Ledger::open_in_memory().unwrap();
        assert_eq!(fresh.schema_sql().unwrap(), golden);

        let run = l.run("01V1RUN0000000000000000000").unwrap().unwrap();
        assert_eq!(run.commit_sha.as_deref(), Some("9490d204a2ba"));
        assert_eq!(run.host.as_deref(), Some("box"));
        assert_eq!(
            (
                run.transport,
                run.step_key,
                run.repo,
                run.outcome,
                run.saved_ms
            ),
            (None, None, None, None, None),
            "later columns are empty for v1 rows"
        );
        let key: Hash = "b3:1111111111111111111111111111111111111111111111111111111111111111"
            .parse()
            .unwrap();
        let cands = l.candidates(&key, 10).unwrap();
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].entries.len(), 1);
        let rs = l.results_for_run("01V1RUN0000000000000000000").unwrap();
        assert_eq!(rs.len(), 1);
        assert_eq!((rs[0].exit_code, rs[0].duration_ms), (0, 1234));
        assert_eq!(l.manifests_for_key(&key).unwrap().len(), 1);
        // No trust state exists for v1 keys: they start observing.
        assert!(l.trust(&key).unwrap().is_none());
        assert!(l.proof(&key, &rs[0].manifest_id).unwrap().is_none());
        assert_eq!(l.runs(None).unwrap().len(), 1);
    }

    #[test]
    fn a_newer_ledger_is_refused() {
        let db = Connection::open_in_memory().unwrap();
        db.pragma_update(None, "user_version", 99).unwrap();
        let mut l = Ledger::wrap(db);
        assert!(matches!(
            l.migrate(),
            Err(LedgerError::TooNew {
                found: 99,
                known: SCHEMA_VERSION
            })
        ));
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

    #[test]
    fn trust_proofs_and_audits_round_trip() {
        let l = Ledger::open_in_memory().unwrap();
        let (key, m1, m2) = (Hash::of(b"key"), Hash::of(b"m1"), Hash::of(b"m2"));
        assert!(l.trust(&key).unwrap().is_none());
        let mut t = TrustRow::new(key, Some("go test ./..."));
        t.eligible = 3;
        l.put_trust(&t).unwrap();
        let back = l.trust(&key).unwrap().unwrap();
        assert_eq!(
            (back.state, back.eligible, back.cmd.as_deref()),
            (TrustState::Observing, 3, Some("go test ./..."))
        );

        let p = |m: Hash, streak: u32, run: &str| ProofRow {
            step_key: key,
            manifest_id: m,
            streak,
            fingerprint: Hash::of(b"fp"),
            last_run: run.into(),
            updated_at: String::new(),
        };
        l.put_proof(&p(m1, 2, "r1")).unwrap();
        l.put_proof(&p(m2, 1, "r2")).unwrap();
        l.put_proof(&p(m2, 5, "r3")).unwrap();
        assert_eq!(l.proof(&key, &m2).unwrap().unwrap().streak, 5);
        let all = l.proofs_for_key(&key).unwrap();
        assert_eq!(
            all.iter()
                .map(|p| (p.manifest_id, p.streak))
                .collect::<Vec<_>>(),
            [(m2, 5), (m1, 2)]
        );
        assert_eq!(l.reset_proofs(&key).unwrap(), 2);
        assert!(
            l.proofs_for_key(&key)
                .unwrap()
                .iter()
                .all(|p| p.streak == 0)
        );
        assert_eq!(
            l.proof(&key, &m1).unwrap().unwrap().fingerprint,
            Hash::of(b"fp"),
            "the fingerprint survives a reset"
        );

        let a = AuditRow {
            id: 0,
            step_key: key,
            manifest_id: m1,
            at: String::new(),
            phase: "audit".into(),
            diverged: true,
            expected_run: Some("r1".into()),
            actual_run: Some("r9".into()),
            detail: Some("exit code 0 -> 1".into()),
        };
        l.record_audit(&a).unwrap();
        l.record_audit(&AuditRow {
            diverged: false,
            phase: "proof".into(),
            ..a.clone()
        })
        .unwrap();
        let got = l.audits(Some(&key), 10).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!((got[0].phase.as_str(), got[0].diverged), ("proof", false));
        assert_eq!((got[1].phase.as_str(), got[1].diverged), ("audit", true));
        assert_eq!(got[1].detail.as_deref(), Some("exit code 0 -> 1"));
        assert_eq!(l.audits(Some(&Hash::of(b"other")), 10).unwrap().len(), 0);
        assert_eq!(l.audits(None, 1).unwrap().len(), 1);
        // Divergences age out of the window: newest first.
        assert_eq!(l.recent_divergences(&key, 10).unwrap(), 1);
        assert_eq!(
            l.recent_divergences(&key, 1).unwrap(),
            0,
            "the newest is clean"
        );
        assert_eq!(l.recent_divergences(&Hash::of(b"other"), 10).unwrap(), 0);
        // One manifest's streak resets, the other's stays.
        l.put_proof(&p(m1, 4, "r4")).unwrap();
        l.put_proof(&p(m2, 3, "r5")).unwrap();
        assert_eq!(l.reset_proof(&key, &m1).unwrap(), 1);
        assert_eq!(l.proof(&key, &m1).unwrap().unwrap().streak, 0);
        assert_eq!(l.proof(&key, &m1).unwrap().unwrap().last_run, "r4");
        assert_eq!(l.proof(&key, &m2).unwrap().unwrap().streak, 3);
    }

    /// `atomically` commits on success, rolls everything back on failure,
    /// and nests by joining the outer transaction. A second connection to
    /// the same file sees only committed state.
    #[test]
    fn atomically_commits_or_rolls_back_as_one() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("ledger.sqlite");
        let l = Ledger::open(&p).unwrap();
        let other = Ledger::open(&p).unwrap();
        let key = Hash::of(b"key");
        let mut t = TrustRow::new(key, None);
        let r: Result<(), LedgerError> = l.atomically(|l| {
            l.put_trust(&t)?;
            l.atomically(|l| l.record_run("r1", None, None, Observed::default()))?;
            assert!(
                other.trust(&key).unwrap().is_none(),
                "not visible before commit"
            );
            Ok(())
        });
        r.unwrap();
        assert!(other.trust(&key).unwrap().is_some());
        assert!(other.run("r1").unwrap().is_some());

        t.state = TrustState::Quarantined;
        let r: Result<(), LedgerError> = l.atomically(|l| {
            l.put_trust(&t)?;
            l.record_run("r2", None, None, Observed::default())?;
            Err(LedgerError::Corrupt("injected".into()))
        });
        assert!(matches!(r, Err(LedgerError::Corrupt(_))));
        assert_eq!(
            other.trust(&key).unwrap().unwrap().state,
            TrustState::Observing,
            "rolled back"
        );
        assert!(other.run("r2").unwrap().is_none());
        // The connection is usable again afterwards.
        l.atomically(|l| l.record_run("r3", None, None, Observed::default()))
            .unwrap();
        assert!(other.run("r3").unwrap().is_some());
    }

    #[test]
    fn run_facts_round_trip_and_filter_by_repo() {
        let l = Ledger::open_in_memory().unwrap();
        let key = Hash::of(b"key");
        let mid = Hash::of(b"m");
        l.record_run_facts(&RunFacts {
            id: "r1",
            commit_sha: Some("c"),
            host: Some("h"),
            observed: Observed {
                transport: Some("fuse"),
                observer: Some("fuse+passthrough"),
            },
            step_key: Some(&key),
            repo: Some("root1"),
            repo_path: Some("/work/a"),
            mode: Some("replay"),
            outcome: Some("replayed"),
            manifest_id: Some(&mid),
            exit_code: Some(0),
            wall_ms: Some(400),
            saved_ms: Some(20_000),
        })
        .unwrap();
        l.record_run_facts(&RunFacts {
            id: "r2",
            repo: Some("root2"),
            outcome: Some("miss"),
            ..RunFacts::default()
        })
        .unwrap();
        let r = l.run("r1").unwrap().unwrap();
        assert_eq!(r.step_key, Some(key));
        assert_eq!(
            (r.transport.as_deref(), r.observer.as_deref()),
            (Some("fuse"), Some("fuse+passthrough"))
        );
        assert_eq!(r.manifest_id, Some(mid));
        assert_eq!(
            (r.outcome.as_deref(), r.saved_ms, r.exit_code),
            (Some("replayed"), Some(20_000), Some(0))
        );
        assert_eq!(l.runs(Some("root1")).unwrap().len(), 1);
        assert_eq!(l.runs(None).unwrap().len(), 2);
        assert_eq!(l.runs(Some("nope")).unwrap().len(), 0);
    }
}
