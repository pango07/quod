//! Quod Cloud's ledger in Postgres (P4, docs/cloud-ledger.md).
//!
//! The same rows as a self-hosted server's SQLite files, in one database:
//! every scope ledger's tables (quod-ledger's schema at `PG_LEDGER_LEVEL`)
//! with `team` and `scope_id` in every key and every query, and the team
//! data of `team.sqlite` (step rows, the event outbox, trusted writes)
//! with `team` in every key. A scope is a row of `scopes`, keyed exactly as
//! the self-hosted directory is (team, repository key, scope), so the
//! SQLite importer maps one to the other.
//!
//! `PgLedger::atomically` keeps `LedgerStore`'s contract exactly: one
//! transaction that first takes a transaction-scoped advisory lock on its
//! scope ledger (team, repository, scope), held to COMMIT or ROLLBACK.
//! Every write to a scope's rows takes the same lock (a write outside
//! `atomically` is its own one-statement `atomically`), so writers of one
//! scope are serialized as SQLite's BEGIN IMMEDIATE serializes writers of
//! one file, across connections and instances. The lock is per scope, a
//! superset of a lock per (team, scope, key): `atomically` is handed no
//! key, and the server's claims and commits already serialize per scope
//! file today. Reads outside `atomically` see committed rows, as SQLite's
//! WAL readers do.
//!
//! Text columns use the "C" collation, so ordering and comparisons
//! (timestamps as RFC 3339 strings, names) are bytewise, as in SQLite.

use std::cell::RefCell;
use std::time::Duration;

use quod_cas::{Hash, Tree};
use quod_ledger::index::{NodeEval, manifest_nodes};
use quod_ledger::{
    AuditRow, Backlog, CANDIDATES, LedgerError, LedgerStore, Lookup, NewPending, ProofRow,
    RunFacts, RunRow, TrustRow, TrustState, now_rfc3339, parse_hash,
};
use quod_manifest::{Manifest, StepResult, matches};
use r2d2_postgres::PostgresConnectionManager;
use r2d2_postgres::postgres::types::ToSql;
use r2d2_postgres::postgres::{Client, Config, NoTls, Row};
use serde_json::Value;

use crate::store::{StepRow, TrustedWrite, EventRow};

type Manager = PostgresConnectionManager<NoTls>;
pub type Pool = r2d2::Pool<Manager>;
type Conn = r2d2::PooledConnection<Manager>;
type P<'a> = &'a (dyn ToSql + Sync);

/// The quod-ledger schema version the Postgres ledger tables hold. The
/// build fails when quod-ledger gains a migration this file has not
/// mirrored (a server must never run a newer ledger's rules on an older
/// ledger's rows).
pub const PG_LEDGER_LEVEL: usize = 8;
const _: () = assert!(PG_LEDGER_LEVEL == quod_ledger::SCHEMA_VERSION);

/// The server's own migrations, in order; each runs once, inside the one
/// transaction `migrate` holds. `TEXT` is spelled `TEXT COLLATE "C"` when
/// they run.
const MIGRATIONS: &[&str] = &[
    // v1: quod-ledger v8 per scope, and team.sqlite v2 per team.
    r#"
CREATE TABLE scopes (
  id BIGSERIAL PRIMARY KEY, team TEXT NOT NULL, repo_key TEXT NOT NULL, repository TEXT NOT NULL,
  scope TEXT NOT NULL, created_at TEXT NOT NULL, UNIQUE (team, repo_key, scope));
CREATE TABLE runs (
  team TEXT NOT NULL, scope_id BIGINT NOT NULL REFERENCES scopes (id), seq BIGSERIAL NOT NULL,
  id TEXT NOT NULL, started_at TEXT NOT NULL, commit_sha TEXT, host TEXT, transport TEXT, observer TEXT,
  step_key TEXT, repo TEXT, repo_path TEXT, mode TEXT, outcome TEXT, manifest_id TEXT, exit_code BIGINT,
  wall_ms BIGINT, saved_ms BIGINT, suspect TEXT, PRIMARY KEY (scope_id, id));
CREATE INDEX runs_by_key ON runs (scope_id, step_key, started_at DESC);
CREATE INDEX runs_by_repo ON runs (scope_id, repo, started_at DESC);
CREATE TABLE manifests (
  team TEXT NOT NULL, scope_id BIGINT NOT NULL REFERENCES scopes (id), seq BIGSERIAL NOT NULL,
  id TEXT NOT NULL, step_key TEXT NOT NULL, run_id TEXT NOT NULL, created_at TEXT NOT NULL,
  json BYTEA NOT NULL, root TEXT, PRIMARY KEY (scope_id, id));
CREATE INDEX manifests_by_key ON manifests (scope_id, step_key, created_at DESC, seq DESC);
CREATE TABLE results (
  team TEXT NOT NULL, scope_id BIGINT NOT NULL REFERENCES scopes (id), seq BIGSERIAL NOT NULL,
  id TEXT NOT NULL, step_key TEXT NOT NULL, manifest_id TEXT NOT NULL, run_id TEXT NOT NULL,
  created_at TEXT NOT NULL, json BYTEA NOT NULL, PRIMARY KEY (scope_id, id));
CREATE INDEX results_by_key ON results (scope_id, step_key, manifest_id, created_at DESC, seq DESC);
CREATE INDEX results_by_run ON results (scope_id, run_id);
CREATE TABLE blobs (
  team TEXT NOT NULL, scope_id BIGINT NOT NULL REFERENCES scopes (id), hash TEXT NOT NULL,
  size BIGINT NOT NULL, path TEXT NOT NULL, PRIMARY KEY (scope_id, hash));
CREATE TABLE step_keys (
  team TEXT NOT NULL, scope_id BIGINT NOT NULL REFERENCES scopes (id), step_key TEXT NOT NULL,
  json BYTEA NOT NULL, PRIMARY KEY (scope_id, step_key));
CREATE TABLE trust (
  team TEXT NOT NULL, scope_id BIGINT NOT NULL REFERENCES scopes (id), step_key TEXT NOT NULL,
  cmd TEXT, state TEXT NOT NULL, eligible BIGINT NOT NULL DEFAULT 0, replays BIGINT NOT NULL DEFAULT 0,
  audits BIGINT NOT NULL DEFAULT 0, divergences BIGINT NOT NULL DEFAULT 0, quarantined_at TEXT,
  quarantine_reason TEXT, updated_at TEXT NOT NULL, shadow_from BIGINT NOT NULL DEFAULT 0,
  volatile BIGINT NOT NULL DEFAULT 0, PRIMARY KEY (scope_id, step_key));
CREATE TABLE proofs (
  team TEXT NOT NULL, scope_id BIGINT NOT NULL REFERENCES scopes (id), step_key TEXT NOT NULL,
  manifest_id TEXT NOT NULL, streak BIGINT NOT NULL, fingerprint TEXT NOT NULL, last_run TEXT NOT NULL,
  updated_at TEXT NOT NULL, PRIMARY KEY (scope_id, step_key, manifest_id));
CREATE TABLE audits (
  id BIGSERIAL PRIMARY KEY, team TEXT NOT NULL, scope_id BIGINT NOT NULL REFERENCES scopes (id),
  step_key TEXT NOT NULL, manifest_id TEXT NOT NULL, at TEXT NOT NULL, phase TEXT NOT NULL,
  diverged BOOLEAN NOT NULL, expected_run TEXT, actual_run TEXT, detail TEXT);
CREATE INDEX audits_by_key ON audits (scope_id, step_key, id DESC);
CREATE INDEX audits_by_scope ON audits (scope_id, id DESC);
CREATE TABLE manifest_nodes (
  team TEXT NOT NULL, id TEXT NOT NULL, body BYTEA NOT NULL, PRIMARY KEY (team, id));
CREATE TABLE audits_pending (
  id BIGSERIAL PRIMARY KEY, team TEXT NOT NULL, scope_id BIGINT NOT NULL REFERENCES scopes (id),
  step_key TEXT NOT NULL, manifest_id TEXT NOT NULL, hit_run TEXT NOT NULL, replay_run TEXT NOT NULL,
  nth BIGINT NOT NULL, tree TEXT NOT NULL, request BYTEA NOT NULL, state TEXT NOT NULL,
  enqueued_at TEXT NOT NULL, attempts BIGINT NOT NULL DEFAULT 0, claimed_at TEXT, claimed_by TEXT,
  finished_at TEXT, audit_id BIGINT, detail TEXT, repaid BOOLEAN NOT NULL DEFAULT false);
CREATE INDEX audits_pending_by_key ON audits_pending (scope_id, step_key, state);
CREATE TABLE network (
  team TEXT NOT NULL, scope_id BIGINT NOT NULL REFERENCES scopes (id), step_key TEXT NOT NULL,
  surface TEXT NOT NULL, reason TEXT NOT NULL, skipped BIGINT NOT NULL DEFAULT 0, updated_at TEXT NOT NULL,
  inputs TEXT, digest TEXT, PRIMARY KEY (scope_id, step_key));
CREATE TABLE steps (
  id BIGSERIAL PRIMARY KEY, team TEXT NOT NULL, at TEXT NOT NULL, repository TEXT NOT NULL,
  scope TEXT NOT NULL, quod_run TEXT NOT NULL, run_id TEXT, run_number BIGINT, run_attempt BIGINT,
  job TEXT, step_index BIGINT, step_name TEXT, step_key TEXT, cmd TEXT, outcome TEXT NOT NULL,
  exit_code BIGINT, wall_ms BIGINT, saved_ms BIGINT, recorded_ms BIGINT, manifest_id TEXT,
  inputs BIGINT, source_run TEXT, streak BIGINT, needed BIGINT, git_ref TEXT, sha TEXT, actor TEXT,
  event TEXT, workflow TEXT, runner TEXT, job_id TEXT, job_name TEXT);
CREATE INDEX steps_by_team ON steps (team, id);
CREATE INDEX steps_by_run ON steps (team, repository, run_number, id);
CREATE INDEX steps_by_quod_run ON steps (team, quod_run);
CREATE TABLE events (
  id BIGSERIAL PRIMARY KEY, team TEXT NOT NULL, at TEXT NOT NULL, type TEXT NOT NULL,
  repository TEXT, run_number BIGINT, step_key TEXT, json TEXT NOT NULL);
CREATE INDEX events_by_team ON events (team, id);
CREATE TABLE trusted_writes (
  team TEXT NOT NULL, repository TEXT NOT NULL, git_ref TEXT NOT NULL, protected BOOLEAN,
  first_at TEXT NOT NULL, last_at TEXT NOT NULL, writes BIGINT NOT NULL,
  PRIMARY KEY (team, repository, git_ref));
CREATE TABLE imports (
  team TEXT PRIMARY KEY, at TEXT NOT NULL, source TEXT NOT NULL, counts TEXT NOT NULL);
"#,
];

/// This build's own schema version (`quod_schema`).
pub const PG_SCHEMA_VERSION: i64 = MIGRATIONS.len() as i64;

/// Advisory lock namespaces: one value per purpose, hashed with its key.
fn lock_key(purpose: &str, parts: &[&str]) -> i64 {
    let mut h = blake3::Hasher::new();
    h.update(b"quod-server pg lock v1\0");
    h.update(purpose.as_bytes());
    for p in parts {
        h.update(b"\0");
        h.update(p.as_bytes());
    }
    let b = h.finalize();
    i64::from_le_bytes(b.as_bytes()[..8].try_into().expect("8 bytes"))
}

/// A Postgres error as a ledger error: the server's code and message
/// (the client's Display is only "db error").
pub fn pg_err(e: r2d2_postgres::postgres::Error) -> LedgerError {
    LedgerError::Store(match e.as_db_error() {
        Some(d) => format!("postgres {}: {}", d.code().code(), d.message()),
        None => format!("postgres: {e}"),
    })
}

fn pool_err(e: r2d2::Error) -> LedgerError {
    LedgerError::Store(format!("postgres pool: {e}"))
}

fn get<T: for<'a> r2d2_postgres::postgres::types::FromSql<'a>>(
    r: &Row,
    i: usize,
) -> Result<T, LedgerError> {
    r.try_get(i).map_err(pg_err)
}

fn u(v: i64) -> u64 {
    v.max(0) as u64
}

fn opt_hash(v: Option<String>) -> Option<Hash> {
    v.and_then(|s| s.parse().ok())
}

/// The pool, connected and migrated.
#[derive(Clone)]
pub struct Pg {
    pub pool: Pool,
}

/// How to reach Postgres.
#[derive(Clone)]
pub struct PgSettings {
    pub url: String,
    pub pool_size: u32,
    /// Extra session options (`-c search_path=...` in tests).
    pub options: Option<String>,
}

impl Pg {
    /// Connects (failing within `connection_timeout` when Postgres cannot
    /// be reached) and brings the schema to this build's version.
    pub fn connect(s: &PgSettings) -> Result<Pg, String> {
        let pg = Pg::open(s)?;
        pg.migrate().map_err(|e| format!("postgres migration: {e}"))?;
        Ok(pg)
    }

    /// The schema version the database records, without changing
    /// anything (0 for a database this server never migrated).
    pub fn recorded_version(&self) -> Result<i64, LedgerError> {
        let mut c = self.conn()?;
        let exists: Option<String> = get(
            &c.query_one("SELECT to_regclass('quod_schema')::text", &[])
                .map_err(pg_err)?,
            0,
        )?;
        if exists.is_none() {
            return Ok(0);
        }
        get(
            &c.query_one("SELECT coalesce(max(version), 0) FROM quod_schema", &[])
                .map_err(pg_err)?,
            0,
        )
    }

    /// Connects without migrating (`quod-server migrate --dry-run`).
    pub fn open(s: &PgSettings) -> Result<Pg, String> {
        let mut cfg: Config = s
            .url
            .parse()
            .map_err(|e| format!("DATABASE_URL does not parse: {e}"))?;
        if matches!(
            cfg.get_ssl_mode(),
            r2d2_postgres::postgres::config::SslMode::Require
        ) {
            return Err("DATABASE_URL asks for sslmode=require, but this build connects without TLS: use the private-network URL (Railway: ${{Postgres.DATABASE_URL}}, whose network is encrypted), not DATABASE_PUBLIC_URL".into());
        }
        // Bounded waits: a statement 2 min, a lock (the scope lock) 60 s,
        // as long as SQLite's busy timeout; a transaction left open by a
        // dead caller is ended by the server.
        let mut opts = String::from(
            "-c statement_timeout=120000 -c lock_timeout=60000 -c idle_in_transaction_session_timeout=300000",
        );
        if let Some(o) = &s.options {
            opts.push(' ');
            opts.push_str(o);
        }
        cfg.options(&opts);
        cfg.application_name("quod-server");
        cfg.connect_timeout(Duration::from_secs(10));
        let pool = r2d2::Pool::builder()
            .max_size(s.pool_size.max(1))
            .min_idle(Some(1))
            .connection_timeout(Duration::from_secs(30))
            .build(PostgresConnectionManager::new(cfg, NoTls))
            .map_err(|e| format!("postgres: cannot connect: {e}"))?;
        Ok(Pg { pool })
    }

    pub fn conn(&self) -> Result<Conn, LedgerError> {
        self.pool.get().map_err(pool_err)
    }

    /// Brings the schema to `PG_SCHEMA_VERSION`: one transaction, under an
    /// advisory lock (a second instance starting at the same time waits,
    /// then finds the work done), each migration applied once and recorded
    /// in `quod_schema`. A database newer than this build is refused, as
    /// quod-ledger refuses a newer ledger. Returns (before, after).
    pub fn migrate(&self) -> Result<(i64, i64), LedgerError> {
        let mut c = self.conn()?;
        let mut tx = c.transaction().map_err(pg_err)?;
        tx.execute(
            "SELECT pg_advisory_xact_lock($1)",
            &[&lock_key("migrate", &[])],
        )
        .map_err(pg_err)?;
        tx.batch_execute(
            "CREATE TABLE IF NOT EXISTS quod_schema (version BIGINT PRIMARY KEY, applied_at TEXT NOT NULL)",
        )
        .map_err(pg_err)?;
        let v: i64 = tx
            .query_one("SELECT coalesce(max(version), 0) FROM quod_schema", &[])
            .map_err(pg_err)?
            .try_get(0)
            .map_err(pg_err)?;
        if v > PG_SCHEMA_VERSION {
            return Err(LedgerError::TooNew {
                found: v as usize,
                known: PG_SCHEMA_VERSION as usize,
            });
        }
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(v as usize) {
            tx.batch_execute(&sql.replace("TEXT", "TEXT COLLATE \"C\""))
                .map_err(pg_err)?;
            tx.execute(
                "INSERT INTO quod_schema (version, applied_at) VALUES ($1, $2)",
                &[&(i as i64 + 1), &now_rfc3339()],
            )
            .map_err(pg_err)?;
        }
        tx.commit().map_err(pg_err)?;
        Ok((v, PG_SCHEMA_VERSION))
    }

    /// The schema version recorded in the database.
    pub fn schema_version(&self) -> Result<i64, LedgerError> {
        let mut c = self.conn()?;
        get(
            &c.query_one("SELECT coalesce(max(version), 0) FROM quod_schema", &[])
                .map_err(pg_err)?,
            0,
        )
    }

    /// A scope's id, creating its row when `create` (and keeping the
    /// repository's newest name, as the self-hosted `repository` file
    /// does).
    pub fn scope_id(
        &self,
        team: &str,
        repo_key: &str,
        repository: &str,
        scope: &str,
        create: bool,
    ) -> Result<Option<i64>, LedgerError> {
        let mut c = self.conn()?;
        let row = c
            .query_opt(
                "SELECT id, repository FROM scopes WHERE team = $1 AND repo_key = $2 AND scope = $3",
                &[&team, &repo_key, &scope],
            )
            .map_err(pg_err)?;
        if let Some(r) = &row {
            let name: String = get(r, 1)?;
            if !create || name == repository {
                return Ok(Some(get(r, 0)?));
            }
        }
        if !create {
            return Ok(None);
        }
        let r = c
            .query_one(
                "INSERT INTO scopes (team, repo_key, repository, scope, created_at) VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT (team, repo_key, scope) DO UPDATE SET repository = EXCLUDED.repository
                 RETURNING id",
                &[&team, &repo_key, &repository, &scope, &now_rfc3339()],
            )
            .map_err(pg_err)?;
        Ok(Some(get(&r, 0)?))
    }

    /// Every scope of a team: (id, repository, scope), by repository then
    /// scope.
    pub fn scopes(&self, team: &str) -> Result<Vec<(i64, String, String)>, LedgerError> {
        let mut c = self.conn()?;
        c.query(
            "SELECT id, repository, scope FROM scopes WHERE team = $1 ORDER BY repository, scope, id",
            &[&team],
        )
        .map_err(pg_err)?
        .iter()
        .map(|r| Ok((get(r, 0)?, get(r, 1)?, get(r, 2)?)))
        .collect()
    }

    /// A scope ledger. It holds a pooled connection only inside
    /// `atomically`; every other statement checks one out and returns it,
    /// so a request never sits on connections it is not using (several
    /// ledgers open at once in one request cannot drain the pool).
    pub fn ledger(&self, team: &str, scope_id: i64) -> Result<PgLedger, LedgerError> {
        Ok(PgLedger {
            pg: self.clone(),
            held: RefCell::new(None),
            team: team.to_string(),
            scope: scope_id,
            lock: lock_key("scope", &[team, &scope_id.to_string()]),
        })
    }

    pub fn activity(&self, team: &str) -> PgActivity {
        PgActivity {
            pg: self.clone(),
            team: team.to_string(),
        }
    }
}

/// One scope ledger in Postgres.
pub struct PgLedger {
    pg: Pg,
    /// The connection of the open transaction (inside `atomically`).
    held: RefCell<Option<Conn>>,
    team: String,
    scope: i64,
    lock: i64,
}

impl Drop for PgLedger {
    fn drop(&mut self) {
        // A transaction left open (a panic inside `atomically`) must not
        // go back to the pool with its lock.
        if let Some(c) = self.held.get_mut() {
            let _ = c.batch_execute("ROLLBACK");
        }
    }
}

const RUN_COLUMNS: &str = "id, started_at, commit_sha, host, transport, observer, step_key, repo, repo_path, mode, outcome, manifest_id, exit_code, wall_ms, saved_ms, suspect";
const TRUST_COLUMNS: &str = "step_key, cmd, state, eligible, replays, audits, divergences, quarantined_at, quarantine_reason, updated_at, shadow_from, volatile";
const PROOF_COLUMNS: &str = "step_key, manifest_id, streak, fingerprint, last_run, updated_at";
const AUDIT_COLUMNS: &str =
    "id, step_key, manifest_id, at, phase, diverged, expected_run, actual_run, detail";

impl PgLedger {
    pub fn scope_id(&self) -> i64 {
        self.scope
    }

    /// Runs `f` on the transaction's connection inside `atomically`, else
    /// on one checked out for it.
    fn with<T>(
        &self,
        f: impl FnOnce(&mut Client) -> Result<T, r2d2_postgres::postgres::Error>,
    ) -> Result<T, LedgerError> {
        let mut held = self.held.borrow_mut();
        if let Some(c) = held.as_mut() {
            return f(c).map_err(pg_err);
        }
        drop(held);
        let mut c = self.pg.conn()?;
        f(&mut c).map_err(pg_err)
    }

    fn query(&self, sql: &str, p: &[P]) -> Result<Vec<Row>, LedgerError> {
        self.with(|c| c.query(sql, p))
    }

    fn query_opt(&self, sql: &str, p: &[P]) -> Result<Option<Row>, LedgerError> {
        self.with(|c| c.query_opt(sql, p))
    }

    fn query_one(&self, sql: &str, p: &[P]) -> Result<Row, LedgerError> {
        self.with(|c| c.query_one(sql, p))
    }

    /// A write: inside the caller's transaction, or its own (which takes
    /// the scope lock, so it never interleaves with an `atomically`).
    fn write(&self, sql: &str, p: &[P]) -> Result<u64, LedgerError> {
        self.atomically(|l| l.with(|c| c.execute(sql, p)))
    }

    fn write_one(&self, sql: &str, p: &[P]) -> Result<Row, LedgerError> {
        self.atomically(|l| l.query_one(sql, p))
    }

    /// A connection with a transaction open and this scope's lock taken.
    fn begin(&self) -> Result<Conn, LedgerError> {
        let mut c = self.pg.conn()?;
        c.batch_execute("BEGIN").map_err(pg_err)?;
        if let Err(e) = c.execute("SELECT pg_advisory_xact_lock($1)", &[&self.lock]) {
            let _ = c.batch_execute("ROLLBACK");
            return Err(pg_err(e));
        }
        Ok(c)
    }

    fn run_row(r: &Row) -> Result<RunRow, LedgerError> {
        Ok(RunRow {
            id: get(r, 0)?,
            started_at: get(r, 1)?,
            commit_sha: get(r, 2)?,
            host: get(r, 3)?,
            transport: get(r, 4)?,
            observer: get(r, 5)?,
            step_key: opt_hash(get(r, 6)?),
            repo: get(r, 7)?,
            repo_path: get(r, 8)?,
            mode: get(r, 9)?,
            outcome: get(r, 10)?,
            manifest_id: opt_hash(get(r, 11)?),
            exit_code: get::<Option<i64>>(r, 12)?.map(|v| v as i32),
            wall_ms: get::<Option<i64>>(r, 13)?.map(u),
            saved_ms: get::<Option<i64>>(r, 14)?.map(u),
            suspect: get(r, 15)?,
        })
    }

    fn trust_row(r: &Row) -> Result<TrustRow, LedgerError> {
        let state: String = get(r, 2)?;
        Ok(TrustRow {
            step_key: parse_hash(&get::<String>(r, 0)?)?,
            cmd: get(r, 1)?,
            state: match state.as_str() {
                "observing" => TrustState::Observing,
                "proven" => TrustState::Proven,
                "quarantined" => TrustState::Quarantined,
                other => return Err(LedgerError::Corrupt(format!("trust state {other:?}"))),
            },
            eligible: u(get(r, 3)?),
            replays: u(get(r, 4)?),
            audits: u(get(r, 5)?),
            divergences: u(get(r, 6)?),
            quarantined_at: get(r, 7)?,
            quarantine_reason: get(r, 8)?,
            updated_at: get(r, 9)?,
            shadow_from: u(get(r, 10)?),
            volatile: u(get(r, 11)?),
        })
    }

    fn proof_row(r: &Row) -> Result<ProofRow, LedgerError> {
        Ok(ProofRow {
            step_key: parse_hash(&get::<String>(r, 0)?)?,
            manifest_id: parse_hash(&get::<String>(r, 1)?)?,
            streak: get::<i64>(r, 2)?.max(0) as u32,
            fingerprint: parse_hash(&get::<String>(r, 3)?)?,
            last_run: get(r, 4)?,
            updated_at: get(r, 5)?,
        })
    }

    fn audit_row(r: &Row) -> Result<AuditRow, LedgerError> {
        Ok(AuditRow {
            id: get(r, 0)?,
            step_key: parse_hash(&get::<String>(r, 1)?)?,
            manifest_id: parse_hash(&get::<String>(r, 2)?)?,
            at: get(r, 3)?,
            phase: get(r, 4)?,
            diverged: get(r, 5)?,
            expected_run: get(r, 6)?,
            actual_run: get(r, 7)?,
            detail: get(r, 8)?,
        })
    }

    pub fn run(&self, id: &str) -> Result<Option<RunRow>, LedgerError> {
        self.query_opt(
            &format!("SELECT {RUN_COLUMNS} FROM runs WHERE scope_id = $1 AND team = $2 AND id = $3"),
            &[&self.scope, &self.team, &id],
        )?
        .as_ref()
        .map(Self::run_row)
        .transpose()
    }

    /// Every run marked suspect, newest first, for one key or (None) all.
    pub fn suspect_runs(&self, step_key: Option<&Hash>) -> Result<Vec<RunRow>, LedgerError> {
        let key = step_key.map(|h| h.to_string());
        self.query(
            &format!(
                "SELECT {RUN_COLUMNS} FROM runs WHERE scope_id = $1 AND team = $2 AND suspect IS NOT NULL
                 AND ($3::text IS NULL OR step_key = $3) ORDER BY started_at DESC, seq DESC"
            ),
            &[&self.scope, &self.team, &key],
        )?
        .iter()
        .map(Self::run_row)
        .collect()
    }

    /// Every trust row, most recently updated first.
    pub fn trust_rows(&self) -> Result<Vec<TrustRow>, LedgerError> {
        self.query(
            &format!("SELECT {TRUST_COLUMNS} FROM trust WHERE scope_id = $1 AND team = $2 ORDER BY updated_at DESC"),
            &[&self.scope, &self.team],
        )?
        .iter()
        .map(Self::trust_row)
        .collect()
    }

    /// Every proof row of a key, longest streak first.
    pub fn proofs_for_key(&self, step_key: &Hash) -> Result<Vec<ProofRow>, LedgerError> {
        self.query(
            &format!("SELECT {PROOF_COLUMNS} FROM proofs WHERE scope_id = $1 AND team = $2 AND step_key = $3 ORDER BY streak DESC, updated_at DESC"),
            &[&self.scope, &self.team, &step_key.to_string()],
        )?
        .iter()
        .map(Self::proof_row)
        .collect()
    }

    /// The newest `limit` audits, for one key or (None) for all keys.
    pub fn audits(
        &self,
        step_key: Option<&Hash>,
        limit: usize,
    ) -> Result<Vec<AuditRow>, LedgerError> {
        let key = step_key.map(|h| h.to_string());
        self.query(
            &format!("SELECT {AUDIT_COLUMNS} FROM audits WHERE scope_id = $1 AND team = $2 AND ($3::text IS NULL OR step_key = $3) ORDER BY id DESC LIMIT $4"),
            &[&self.scope, &self.team, &key, &(limit as i64)],
        )?
        .iter()
        .map(Self::audit_row)
        .collect()
    }

    /// Deferred-audit rows of this scope in any state (a server never
    /// holds one: test S15).
    pub fn pending_total(&self) -> Result<u64, LedgerError> {
        Ok(u(get(
            &self.query_one(
                "SELECT count(*) FROM audits_pending WHERE scope_id = $1 AND team = $2",
                &[&self.scope, &self.team],
            )?,
            0,
        )?))
    }

    fn manifest_count(&self, step_key: &Hash) -> Result<usize, LedgerError> {
        Ok(u(get(
            &self.query_one(
                "SELECT count(*) FROM manifests WHERE scope_id = $1 AND team = $2 AND step_key = $3",
                &[&self.scope, &self.team, &step_key.to_string()],
            )?,
            0,
        )?) as usize)
    }

    fn node(&self, id: &Hash) -> Result<Option<Vec<u8>>, LedgerError> {
        self.query_opt(
            "SELECT body FROM manifest_nodes WHERE team = $1 AND id = $2",
            &[&self.team, &id.to_string()],
        )?
        .map(|r| get(&r, 0))
        .transpose()
    }

    fn put_nodes(&self, m: &Manifest) -> Result<Hash, LedgerError> {
        let (root, nodes) = manifest_nodes(m)?;
        let (ids, bodies): (Vec<String>, Vec<Vec<u8>>) = nodes
            .into_iter()
            .map(|(id, body)| (id.to_string(), body.into_bytes()))
            .unzip();
        self.write(
            "INSERT INTO manifest_nodes (team, id, body) SELECT $1, n.id, n.body FROM UNNEST($2::text[], $3::bytea[]) AS n (id, body)
             ON CONFLICT (team, id) DO NOTHING",
            &[&self.team, &ids, &bodies],
        )?;
        Ok(root)
    }

    fn insert_run(&self, f: &RunFacts) -> Result<(), LedgerError> {
        self.write(
            "INSERT INTO runs (team, scope_id, id, started_at, commit_sha, host, transport, observer, step_key, repo, repo_path, mode, outcome, manifest_id, exit_code, wall_ms, saved_ms)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)",
            &[
                &self.team,
                &self.scope,
                &f.id,
                &now_rfc3339(),
                &f.commit_sha,
                &f.host,
                &f.observed.transport,
                &f.observed.observer,
                &f.step_key.map(|h| h.to_string()),
                &f.repo,
                &f.repo_path,
                &f.mode,
                &f.outcome,
                &f.manifest_id.map(|h| h.to_string()),
                &f.exit_code.map(i64::from),
                &f.wall_ms.map(|v| v as i64),
                &f.saved_ms.map(|v| v as i64),
            ],
        )?;
        Ok(())
    }

    fn insert_manifest(&self, m: &Manifest, run_id: &str) -> Result<Hash, LedgerError> {
        let json = m.canonical_json()?;
        let id = Hash::of(json.as_bytes());
        let root = self.put_nodes(m)?;
        // Re-recording refreshes its position as newest (SQLite's REPLACE
        // gives it a new rowid; here a new `seq`).
        self.write(
            "INSERT INTO manifests (team, scope_id, id, step_key, run_id, created_at, json, root) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (scope_id, id) DO UPDATE SET step_key = EXCLUDED.step_key, run_id = EXCLUDED.run_id,
               created_at = EXCLUDED.created_at, json = EXCLUDED.json, root = EXCLUDED.root, seq = DEFAULT",
            &[
                &self.team,
                &self.scope,
                &id.to_string(),
                &m.step_key.to_string(),
                &run_id,
                &now_rfc3339(),
                &json.into_bytes(),
                &root.to_string(),
            ],
        )?;
        Ok(id)
    }

    fn insert_result(&self, r: &StepResult) -> Result<(), LedgerError> {
        let json = quod_cas::canonical_json(r).map_err(quod_manifest::ManifestError::from)?;
        self.write(
            "INSERT INTO results (team, scope_id, id, step_key, manifest_id, run_id, created_at, json) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            &[
                &self.team,
                &self.scope,
                &r.run_id,
                &r.step_key.to_string(),
                &r.manifest_id.to_string(),
                &r.run_id,
                &now_rfc3339(),
                &json.into_bytes(),
            ],
        )?;
        Ok(())
    }

    /// Indexes a manifest stored without its root (imported from a ledger
    /// that had not indexed it yet); None if the manifest is gone.
    fn index_manifest(&self, id: &Hash) -> Result<Option<Hash>, LedgerError> {
        let Some(m) = self.manifest(id)? else {
            return Ok(None);
        };
        let root = self.put_nodes(&m)?;
        self.write(
            "UPDATE manifests SET root = $3 WHERE scope_id = $1 AND team = $2 AND id = $4",
            &[&self.scope, &self.team, &root.to_string(), &id.to_string()],
        )?;
        Ok(Some(root))
    }
}

impl LedgerStore for PgLedger {
    fn atomically<T, E: From<LedgerError>>(
        &self,
        f: impl FnOnce(&Self) -> Result<T, E>,
    ) -> Result<T, E> {
        if self.held.borrow().is_some() {
            return f(self);
        }
        let c = self.begin()?;
        *self.held.borrow_mut() = Some(c);
        let out = f(self);
        let mut c = self
            .held
            .borrow_mut()
            .take()
            .expect("the transaction's connection");
        match out {
            Ok(v) => {
                c.batch_execute("COMMIT").map_err(pg_err)?;
                Ok(v)
            }
            Err(e) => {
                let _ = c.batch_execute("ROLLBACK");
                Err(e)
            }
        }
    }

    /// `Ledger::find_match`, the same algorithm over the same rows: the
    /// newest candidate decoded and checked whole, the older ones through
    /// the index (`NodeEval`), each index hit checked again in full with
    /// `matches`; a manifest or result from a newer schema never matches.
    fn find_match(&self, step_key: &Hash, tree: &Tree) -> Result<Lookup, LedgerError> {
        let key = step_key.to_string();
        let rows: Vec<(String, Option<String>)> = self
            .query(
                "SELECT id, root FROM manifests WHERE scope_id = $1 AND team = $2 AND step_key = $3
                 ORDER BY created_at DESC, seq DESC LIMIT $4",
                &[&self.scope, &self.team, &key, &(CANDIDATES as i64)],
            )?
            .iter()
            .map(|r| Ok((get(r, 0)?, get(r, 1)?)))
            .collect::<Result<_, LedgerError>>()?;
        let total = self.manifest_count(step_key)?;
        let mut closest = None;
        if let Some((id, _)) = rows.first() {
            let mid = parse_hash(id)?;
            if let Some(m) = self.manifest(&mid)?
                && m.v <= quod_manifest::VERSION
            {
                match matches(&m, tree) {
                    Ok(()) => {
                        if let Some(r) = self.latest_result(step_key, &mid)?
                            && r.v <= quod_manifest::VERSION
                        {
                            return Ok(Lookup {
                                hit: Some((m, r)),
                                candidates: total,
                                closest: None,
                            });
                        }
                    }
                    Err(mm) => closest = Some((mid, *mm)),
                }
            }
        }
        let unindexed: Vec<Hash> = rows
            .iter()
            .skip(1)
            .filter(|(_, root)| root.is_none())
            .map(|(id, _)| parse_hash(id))
            .collect::<Result<_, _>>()?;
        let mut indexed = std::collections::HashMap::new();
        if !unindexed.is_empty() {
            self.atomically(|l| -> Result<(), LedgerError> {
                for mid in &unindexed {
                    if let Some(root) = l.index_manifest(mid)? {
                        indexed.insert(*mid, root);
                    }
                }
                Ok(())
            })?;
        }
        let mut eval = NodeEval::new(tree, Box::new(|id: &Hash| self.node(id)));
        let mut hit = None;
        for (id, root) in rows.iter().skip(1) {
            let mid = parse_hash(id)?;
            let root = match root {
                Some(r) => parse_hash(r)?,
                None => match indexed.get(&mid) {
                    Some(r) => *r,
                    None => continue,
                },
            };
            if !eval.matches(&root)? {
                continue;
            }
            let Some(m) = self.manifest(&mid)? else {
                continue;
            };
            if m.v > quod_manifest::VERSION || matches(&m, tree).is_err() {
                continue;
            }
            if let Some(r) = self.latest_result(step_key, &mid)?
                && r.v <= quod_manifest::VERSION
            {
                hit = Some((m, r));
                break;
            }
        }
        Ok(Lookup {
            hit,
            candidates: total,
            closest,
        })
    }

    fn manifest(&self, id: &Hash) -> Result<Option<Manifest>, LedgerError> {
        match self.query_opt(
            "SELECT json FROM manifests WHERE scope_id = $1 AND team = $2 AND id = $3",
            &[&self.scope, &self.team, &id.to_string()],
        )? {
            Some(r) => Ok(Some(serde_json::from_slice(&get::<Vec<u8>>(&r, 0)?)?)),
            None => Ok(None),
        }
    }

    fn latest_result(
        &self,
        step_key: &Hash,
        manifest_id: &Hash,
    ) -> Result<Option<StepResult>, LedgerError> {
        match self.query_opt(
            "SELECT json FROM results WHERE scope_id = $1 AND team = $2 AND step_key = $3 AND manifest_id = $4
             ORDER BY created_at DESC, seq DESC LIMIT 1",
            &[
                &self.scope,
                &self.team,
                &step_key.to_string(),
                &manifest_id.to_string(),
            ],
        )? {
            Some(r) => Ok(Some(serde_json::from_slice(&get::<Vec<u8>>(&r, 0)?)?)),
            None => Ok(None),
        }
    }

    fn results_for_run(&self, run_id: &str) -> Result<Vec<StepResult>, LedgerError> {
        self.query(
            "SELECT json FROM results WHERE scope_id = $1 AND team = $2 AND run_id = $3 ORDER BY seq",
            &[&self.scope, &self.team, &run_id],
        )?
        .iter()
        .map(|r| Ok(serde_json::from_slice(&get::<Vec<u8>>(r, 0)?)?))
        .collect()
    }

    fn put_step_key(&self, step_key: &Hash, json: &str) -> Result<(), LedgerError> {
        self.write(
            "INSERT INTO step_keys (team, scope_id, step_key, json) VALUES ($1, $2, $3, $4) ON CONFLICT (scope_id, step_key) DO NOTHING",
            &[&self.team, &self.scope, &step_key.to_string(), &json.as_bytes()],
        )?;
        Ok(())
    }

    fn record_run_facts(&self, f: &RunFacts) -> Result<(), LedgerError> {
        self.insert_run(f)
    }

    fn record_execution_facts(
        &self,
        run: &RunFacts,
        m: &Manifest,
        r: &StepResult,
    ) -> Result<Hash, LedgerError> {
        self.atomically(|l| {
            l.insert_run(run)?;
            let id = l.insert_manifest(m, run.id)?;
            l.insert_result(r)?;
            Ok(id)
        })
    }

    fn trust(&self, step_key: &Hash) -> Result<Option<TrustRow>, LedgerError> {
        self.query_opt(
            &format!("SELECT {TRUST_COLUMNS} FROM trust WHERE scope_id = $1 AND team = $2 AND step_key = $3"),
            &[&self.scope, &self.team, &step_key.to_string()],
        )?
        .as_ref()
        .map(Self::trust_row)
        .transpose()
    }

    fn put_trust(&self, t: &TrustRow) -> Result<(), LedgerError> {
        self.write(
            "INSERT INTO trust (team, scope_id, step_key, cmd, state, eligible, replays, audits, divergences, quarantined_at, quarantine_reason, updated_at, shadow_from, volatile)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
             ON CONFLICT (scope_id, step_key) DO UPDATE SET cmd = EXCLUDED.cmd, state = EXCLUDED.state,
               eligible = EXCLUDED.eligible, replays = EXCLUDED.replays, audits = EXCLUDED.audits,
               divergences = EXCLUDED.divergences, quarantined_at = EXCLUDED.quarantined_at,
               quarantine_reason = EXCLUDED.quarantine_reason, updated_at = EXCLUDED.updated_at,
               shadow_from = EXCLUDED.shadow_from, volatile = EXCLUDED.volatile",
            &[
                &self.team,
                &self.scope,
                &t.step_key.to_string(),
                &t.cmd,
                &t.state.as_str(),
                &(t.eligible as i64),
                &(t.replays as i64),
                &(t.audits as i64),
                &(t.divergences as i64),
                &t.quarantined_at,
                &t.quarantine_reason,
                &now_rfc3339(),
                &(t.shadow_from as i64),
                &(t.volatile as i64),
            ],
        )?;
        Ok(())
    }

    fn proof(&self, step_key: &Hash, manifest_id: &Hash) -> Result<Option<ProofRow>, LedgerError> {
        self.query_opt(
            &format!("SELECT {PROOF_COLUMNS} FROM proofs WHERE scope_id = $1 AND team = $2 AND step_key = $3 AND manifest_id = $4"),
            &[
                &self.scope,
                &self.team,
                &step_key.to_string(),
                &manifest_id.to_string(),
            ],
        )?
        .as_ref()
        .map(Self::proof_row)
        .transpose()
    }

    fn put_proof(&self, p: &ProofRow) -> Result<(), LedgerError> {
        self.write(
            "INSERT INTO proofs (team, scope_id, step_key, manifest_id, streak, fingerprint, last_run, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (scope_id, step_key, manifest_id) DO UPDATE SET streak = EXCLUDED.streak,
               fingerprint = EXCLUDED.fingerprint, last_run = EXCLUDED.last_run, updated_at = EXCLUDED.updated_at",
            &[
                &self.team,
                &self.scope,
                &p.step_key.to_string(),
                &p.manifest_id.to_string(),
                &i64::from(p.streak),
                &p.fingerprint.to_string(),
                &p.last_run,
                &now_rfc3339(),
            ],
        )?;
        Ok(())
    }

    fn reset_proofs(&self, step_key: &Hash) -> Result<usize, LedgerError> {
        Ok(self.write(
            "UPDATE proofs SET streak = 0, updated_at = $4 WHERE scope_id = $1 AND team = $2 AND step_key = $3",
            &[&self.scope, &self.team, &step_key.to_string(), &now_rfc3339()],
        )? as usize)
    }

    fn reset_proof(&self, step_key: &Hash, manifest_id: &Hash) -> Result<usize, LedgerError> {
        Ok(self.write(
            "UPDATE proofs SET streak = 0, updated_at = $5 WHERE scope_id = $1 AND team = $2 AND step_key = $3 AND manifest_id = $4",
            &[
                &self.scope,
                &self.team,
                &step_key.to_string(),
                &manifest_id.to_string(),
                &now_rfc3339(),
            ],
        )? as usize)
    }

    fn recent_divergences(&self, step_key: &Hash, window: usize) -> Result<u64, LedgerError> {
        Ok(u(get(
            &self.query_one(
                "SELECT count(*) FROM (SELECT diverged FROM audits WHERE scope_id = $1 AND team = $2 AND step_key = $3
                 ORDER BY id DESC LIMIT $4) w WHERE diverged",
                &[
                    &self.scope,
                    &self.team,
                    &step_key.to_string(),
                    &(window as i64),
                ],
            )?,
            0,
        )?))
    }

    fn record_audit(&self, a: &AuditRow) -> Result<i64, LedgerError> {
        get(
            &self.write_one(
                "INSERT INTO audits (team, scope_id, step_key, manifest_id, at, phase, diverged, expected_run, actual_run, detail)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING id",
                &[
                    &self.team,
                    &self.scope,
                    &a.step_key.to_string(),
                    &a.manifest_id.to_string(),
                    &now_rfc3339(),
                    &a.phase,
                    &a.diverged,
                    &a.expected_run,
                    &a.actual_run,
                    &a.detail,
                ],
            )?,
            0,
        )
    }

    fn mark_suspect_replays(&self, step_key: &Hash, why: &str) -> Result<usize, LedgerError> {
        Ok(self.write(
            "UPDATE runs SET suspect = $4 WHERE scope_id = $1 AND team = $2 AND step_key = $3 AND outcome = 'replayed' AND suspect IS NULL",
            &[&self.scope, &self.team, &step_key.to_string(), &why],
        )? as usize)
    }

    fn backlog(&self, step_key: &Hash, stale_before: &str) -> Result<Backlog, LedgerError> {
        let r = self.query_one(
            "SELECT
               count(*) FILTER (WHERE state = 'running' OR (state = 'pending' AND enqueued_at >= $4)),
               count(*) FILTER (WHERE (state = 'lost' AND NOT repaid) OR (state = 'pending' AND enqueued_at < $4))
             FROM audits_pending WHERE scope_id = $1 AND team = $2 AND step_key = $3",
            &[&self.scope, &self.team, &step_key.to_string(), &stale_before],
        )?;
        Ok(Backlog {
            pending: u(get(&r, 0)?),
            owed: u(get(&r, 1)?),
        })
    }

    fn enqueue_audit(&self, a: &NewPending) -> Result<i64, LedgerError> {
        get(
            &self.write_one(
                "INSERT INTO audits_pending (team, scope_id, step_key, manifest_id, hit_run, replay_run, nth, tree, request, state, enqueued_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'pending', $10) RETURNING id",
                &[
                    &self.team,
                    &self.scope,
                    &a.step_key.to_string(),
                    &a.manifest_id.to_string(),
                    &a.hit_run,
                    &a.replay_run,
                    &(a.nth as i64),
                    &a.tree.to_string(),
                    &a.request,
                    &now_rfc3339(),
                ],
            )?,
            0,
        )
    }

    fn finish_audit(
        &self,
        id: i64,
        diverged: bool,
        audit_id: i64,
        detail: &str,
    ) -> Result<(), LedgerError> {
        self.write(
            "UPDATE audits_pending SET state = $4, finished_at = $5, audit_id = $6, detail = $7 WHERE scope_id = $1 AND team = $2 AND id = $3",
            &[
                &self.scope,
                &self.team,
                &id,
                &if diverged { "diverged" } else { "passed" },
                &now_rfc3339(),
                &audit_id,
                &detail,
            ],
        )?;
        Ok(())
    }

    fn supersede_pending(
        &self,
        step_key: &Hash,
        except: Option<i64>,
        why: &str,
    ) -> Result<usize, LedgerError> {
        Ok(self.write(
            "UPDATE audits_pending SET state = 'lost', repaid = true, finished_at = $4, detail = $5
             WHERE scope_id = $1 AND team = $2 AND step_key = $3 AND state IN ('pending', 'running')
               AND ($6::bigint IS NULL OR id <> $6)",
            &[
                &self.scope,
                &self.team,
                &step_key.to_string(),
                &now_rfc3339(),
                &why,
                &except,
            ],
        )? as usize)
    }

    fn repay_audit(
        &self,
        step_key: &Hash,
        audit_id: i64,
        stale_before: &str,
    ) -> Result<bool, LedgerError> {
        self.atomically(|l| {
            let Some(r) = l.query_opt(
                "SELECT id, state, enqueued_at, detail FROM audits_pending
                 WHERE scope_id = $1 AND team = $2 AND step_key = $3
                   AND ((state = 'lost' AND NOT repaid) OR (state = 'pending' AND enqueued_at < $4))
                 ORDER BY id LIMIT 1",
                &[&l.scope, &l.team, &step_key.to_string(), &stale_before],
            )?
            else {
                return Ok(false);
            };
            let id: i64 = get(&r, 0)?;
            let state: String = get(&r, 1)?;
            let enqueued_at: String = get(&r, 2)?;
            let detail = if state == "pending" {
                format!("not executed since {enqueued_at} (overdue; repaid by audit {audit_id})")
            } else {
                format!(
                    "{} (repaid by audit {audit_id})",
                    get::<Option<String>>(&r, 3)?.unwrap_or_default()
                )
            };
            l.write(
                "UPDATE audits_pending SET state = 'lost', repaid = true, finished_at = coalesce(finished_at, $4), detail = $5
                 WHERE scope_id = $1 AND team = $2 AND id = $3",
                &[&l.scope, &l.team, &id, &now_rfc3339(), &detail],
            )?;
            Ok(true)
        })
    }
}

/// A team's step rows, event outbox and trusted writes (team.sqlite's
/// tables), in Postgres.
pub struct PgActivity {
    pg: Pg,
    team: String,
}

fn step_row(r: &Row) -> Result<StepRow, LedgerError> {
    let uo = |i| get::<Option<i64>>(r, i).map(|v| v.map(u));
    let u32o = |i| get::<Option<i64>>(r, i).map(|v| v.map(|v| v.max(0) as u32));
    Ok(StepRow {
        at: get(r, 0)?,
        repository: get(r, 1)?,
        scope: get(r, 2)?,
        quod_run: get(r, 3)?,
        run_id: get(r, 4)?,
        run_number: uo(5)?,
        run_attempt: uo(6)?,
        job: get(r, 7)?,
        step_index: u32o(8)?,
        step_name: get(r, 9)?,
        step_key: get(r, 10)?,
        cmd: get(r, 11)?,
        outcome: get(r, 12)?,
        exit_code: get::<Option<i64>>(r, 13)?.map(|v| v as i32),
        wall_ms: uo(14)?,
        saved_ms: uo(15)?,
        recorded_ms: uo(16)?,
        manifest_id: get(r, 17)?,
        inputs: uo(18)?,
        source_run: get(r, 19)?,
        streak: u32o(20)?,
        needed: u32o(21)?,
        git_ref: get(r, 22)?,
        sha: get(r, 23)?,
        actor: get(r, 24)?,
        event: get(r, 25)?,
        workflow: get(r, 26)?,
        runner: get(r, 27)?,
        job_id: get(r, 28)?,
        job_name: get(r, 29)?,
    })
}

const STEP_COLUMNS: &str = "at, repository, scope, quod_run, run_id, run_number, run_attempt, job, step_index, step_name, step_key, cmd, outcome, exit_code, wall_ms, saved_ms, recorded_ms, manifest_id, inputs, source_run, streak, needed, git_ref, sha, actor, event, workflow, runner, job_id, job_name";

impl PgActivity {
    pub fn insert_step(&self, s: &StepRow) -> Result<(), LedgerError> {
        let i = |v: Option<u64>| v.map(|v| v as i64);
        let i32o = |v: Option<u32>| v.map(i64::from);
        self.pg
            .conn()?
            .execute(
                &format!(
                    "INSERT INTO steps (team, {STEP_COLUMNS}) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23,$24,$25,$26,$27,$28,$29,$30,$31)"
                ),
                &[
                    &self.team,
                    &s.at,
                    &s.repository,
                    &s.scope,
                    &s.quod_run,
                    &s.run_id,
                    &i(s.run_number),
                    &i(s.run_attempt),
                    &s.job,
                    &i32o(s.step_index),
                    &s.step_name,
                    &s.step_key,
                    &s.cmd,
                    &s.outcome,
                    &s.exit_code.map(i64::from),
                    &i(s.wall_ms),
                    &i(s.saved_ms),
                    &i(s.recorded_ms),
                    &s.manifest_id,
                    &i(s.inputs),
                    &s.source_run,
                    &i32o(s.streak),
                    &i32o(s.needed),
                    &s.git_ref,
                    &s.sha,
                    &s.actor,
                    &s.event,
                    &s.workflow,
                    &s.runner,
                    &s.job_id,
                    &s.job_name,
                ],
            )
            .map_err(pg_err)?;
        Ok(())
    }

    pub fn record_trusted_write(
        &self,
        repository: &str,
        git_ref: &str,
        protected: Option<bool>,
    ) -> Result<(), LedgerError> {
        self.pg
            .conn()?
            .execute(
                "INSERT INTO trusted_writes (team, repository, git_ref, protected, first_at, last_at, writes) VALUES ($1, $2, $3, $4, $5, $5, 1)
                 ON CONFLICT (team, repository, git_ref) DO UPDATE SET protected = EXCLUDED.protected,
                   last_at = EXCLUDED.last_at, writes = trusted_writes.writes + 1",
                &[&self.team, &repository, &git_ref, &protected, &now_rfc3339()],
            )
            .map_err(pg_err)?;
        Ok(())
    }

    pub fn trusted_writes(
        &self,
        repository: Option<&str>,
    ) -> Result<Vec<TrustedWrite>, LedgerError> {
        self.pg
            .conn()?
            .query(
                "SELECT repository, git_ref, protected, first_at, last_at, writes FROM trusted_writes
                 WHERE team = $1 AND ($2::text IS NULL OR repository = $2) ORDER BY repository, git_ref",
                &[&self.team, &repository],
            )
            .map_err(pg_err)?
            .iter()
            .map(|r| {
                Ok(TrustedWrite {
                    repository: get(r, 0)?,
                    git_ref: get(r, 1)?,
                    protected: get(r, 2)?,
                    first_at: get(r, 3)?,
                    last_at: get(r, 4)?,
                    writes: u(get(r, 5)?),
                })
            })
            .collect()
    }

    pub fn run_number_of(&self, quod_run: &str) -> Result<Option<u64>, LedgerError> {
        Ok(self
            .pg
            .conn()?
            .query_opt(
                "SELECT run_number FROM steps WHERE team = $1 AND quod_run = $2 ORDER BY id DESC LIMIT 1",
                &[&self.team, &quod_run],
            )
            .map_err(pg_err)?
            .map(|r| get::<Option<i64>>(&r, 0))
            .transpose()?
            .flatten()
            .map(u))
    }

    pub fn steps(&self, repository: Option<&str>) -> Result<Vec<StepRow>, LedgerError> {
        self.pg
            .conn()?
            .query(
                &format!("SELECT {STEP_COLUMNS} FROM steps WHERE team = $1 AND ($2::text IS NULL OR repository = $2) ORDER BY id"),
                &[&self.team, &repository],
            )
            .map_err(pg_err)?
            .iter()
            .map(step_row)
            .collect()
    }

    /// Appends an event; returns its id. Under the team's event lock, so
    /// a team's events commit in id order and a reader resuming after an
    /// id never skips one that commits later with a smaller id.
    pub fn push_event(
        &self,
        kind: &str,
        repository: Option<&str>,
        run_number: Option<u64>,
        step_key: Option<&str>,
        json: &Value,
    ) -> Result<i64, LedgerError> {
        let mut c = self.pg.conn()?;
        let mut tx = c.transaction().map_err(pg_err)?;
        tx.execute(
            "SELECT pg_advisory_xact_lock($1)",
            &[&lock_key("events", &[&self.team])],
        )
        .map_err(pg_err)?;
        let id: i64 = get(
            &tx.query_one(
                "INSERT INTO events (team, at, type, repository, run_number, step_key, json) VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id",
                &[
                    &self.team,
                    &now_rfc3339(),
                    &kind,
                    &repository,
                    &run_number.map(|v| v as i64),
                    &step_key,
                    &json.to_string(),
                ],
            )
            .map_err(pg_err)?,
            0,
        )?;
        tx.commit().map_err(pg_err)?;
        Ok(id)
    }

    pub fn events_after(&self, after: i64, limit: usize) -> Result<Vec<EventRow>, LedgerError> {
        self.pg
            .conn()?
            .query(
                "SELECT id, type, repository, run_number, step_key, json FROM events WHERE team = $1 AND id > $2 ORDER BY id LIMIT $3",
                &[&self.team, &after, &(limit as i64)],
            )
            .map_err(pg_err)?
            .iter()
            .map(|r| {
                Ok(EventRow {
                    id: get(r, 0)?,
                    kind: get(r, 1)?,
                    repository: get(r, 2)?,
                    run_number: get::<Option<i64>>(r, 3)?.map(u),
                    step_key: get(r, 4)?,
                    json: serde_json::from_str(&get::<String>(r, 5)?)?,
                })
            })
            .collect()
    }

    pub fn last_event(&self) -> Result<i64, LedgerError> {
        get(
            &self
                .pg
                .conn()?
                .query_one(
                    "SELECT coalesce(max(id), 0) FROM events WHERE team = $1",
                    &[&self.team],
                )
                .map_err(pg_err)?,
            0,
        )
    }
}
