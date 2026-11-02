//! Where a team's data lives on a self-hosted server: one directory per
//! team, so no query can reach another team's rows or blobs by mistake.
//!
//! ```text
//! <data>/teams/<team>/cas/                         blobs (quod_cas::Cas)
//! <data>/teams/<team>/uploads/                     resumable uploads in progress
//! <data>/teams/<team>/scopes/<repo>/<trust>/ledger.sqlite
//! <data>/teams/<team>/team.sqlite                  CI activity and the event outbox
//! ```
//!
//! `<repo>` is the repository's readable name plus a hash of it (two names
//! never share a directory); its `repository` file holds the name. The
//! scope ledgers are ordinary quod ledgers (schema v4), the same files a
//! runner keeps locally. Postgres and an object store replace this layout
//! in P4 (docs/cloud-ledger.md) behind the same LedgerStore and BlobStore
//! traits, with the team id a column every query filters on.

use std::path::{Path, PathBuf};

use quod_cas::Cas;
use quod_ledger::Ledger;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

#[derive(Clone, Debug)]
pub struct Store {
    root: PathBuf,
}

/// One scope ledger of a team.
#[derive(Clone, Debug)]
pub struct ScopeRef {
    pub repository: String,
    pub scope: String,
    pub path: PathBuf,
}

impl Store {
    pub fn new(root: impl Into<PathBuf>) -> Store {
        Store { root: root.into() }
    }

    pub fn team_dir(&self, team: &str) -> PathBuf {
        debug_assert!(crate::valid_id(team));
        self.root.join("teams").join(team)
    }

    pub fn cas(&self, team: &str) -> anyhow::Result<Cas> {
        Ok(Cas::open(self.team_dir(team).join("cas"))?)
    }

    pub fn uploads(&self, team: &str) -> anyhow::Result<PathBuf> {
        let d = self.team_dir(team).join("uploads");
        std::fs::create_dir_all(&d)?;
        Ok(d)
    }

    fn repo_dir(&self, team: &str, repository: &str) -> PathBuf {
        let readable: String = repository
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
                    c
                } else {
                    '_'
                }
            })
            .take(80)
            .collect();
        let h = &blake3::hash(repository.as_bytes()).to_hex()[..12];
        self.team_dir(team)
            .join("scopes")
            .join(format!("{readable}-{h}"))
    }

    fn ledger_path(&self, team: &str, repository: &str, scope: &str) -> PathBuf {
        debug_assert!(crate::valid_id(scope));
        self.repo_dir(team, repository)
            .join(scope)
            .join("ledger.sqlite")
    }

    pub fn has_ledger(&self, team: &str, repository: &str, scope: &str) -> bool {
        self.ledger_path(team, repository, scope).is_file()
    }

    /// Opens (creating) a scope's ledger.
    pub fn ledger(&self, team: &str, repository: &str, scope: &str) -> anyhow::Result<Ledger> {
        let p = self.ledger_path(team, repository, scope);
        let repo_dir = self.repo_dir(team, repository);
        std::fs::create_dir_all(p.parent().unwrap())?;
        let name = repo_dir.join("repository");
        if !name.is_file() {
            std::fs::write(&name, repository)?;
        }
        Ok(Ledger::open(&p)?)
    }

    /// Every scope ledger of a team.
    pub fn scopes(&self, team: &str) -> Vec<ScopeRef> {
        let mut out = vec![];
        let Ok(repos) = std::fs::read_dir(self.team_dir(team).join("scopes")) else {
            return out;
        };
        for r in repos.flatten() {
            let Ok(repository) = std::fs::read_to_string(r.path().join("repository")) else {
                continue;
            };
            let Ok(scopes) = std::fs::read_dir(r.path()) else {
                continue;
            };
            for s in scopes.flatten() {
                let path = s.path().join("ledger.sqlite");
                if path.is_file() {
                    out.push(ScopeRef {
                        repository: repository.clone(),
                        scope: s.file_name().to_string_lossy().into_owned(),
                        path,
                    });
                }
            }
        }
        out.sort_by(|a, b| (&a.repository, &a.scope).cmp(&(&b.repository, &b.scope)));
        out
    }

    pub fn activity(&self, team: &str) -> anyhow::Result<Activity> {
        let d = self.team_dir(team);
        std::fs::create_dir_all(&d)?;
        Activity::open(&d.join("team.sqlite"))
    }
}

/// A team's CI activity (one row per step a runner reported) and its event
/// outbox (the dashboard's stream reads it by id, which makes resuming with
/// Last-Event-ID exact).
pub struct Activity {
    db: Connection,
}

const ACTIVITY_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS steps (
  id INTEGER PRIMARY KEY AUTOINCREMENT, at TEXT NOT NULL, repository TEXT NOT NULL, scope TEXT NOT NULL,
  quod_run TEXT NOT NULL, run_id TEXT, run_number INTEGER, run_attempt INTEGER, job TEXT, step_index INTEGER,
  step_name TEXT, step_key TEXT, cmd TEXT, outcome TEXT NOT NULL, exit_code INTEGER, wall_ms INTEGER,
  saved_ms INTEGER, recorded_ms INTEGER, manifest_id TEXT, inputs INTEGER, source_run TEXT, streak INTEGER,
  needed INTEGER, git_ref TEXT, sha TEXT, actor TEXT, event TEXT, workflow TEXT, runner TEXT);
CREATE INDEX IF NOT EXISTS steps_by_run ON steps (repository, run_number, id);
CREATE TABLE IF NOT EXISTS events (
  id INTEGER PRIMARY KEY AUTOINCREMENT, at TEXT NOT NULL, type TEXT NOT NULL, repository TEXT,
  run_number INTEGER, step_key TEXT, json TEXT NOT NULL);
";

/// One step row.
#[derive(Clone, Debug, Default)]
pub struct StepRow {
    pub at: String,
    pub repository: String,
    pub scope: String,
    pub quod_run: String,
    pub run_id: Option<String>,
    pub run_number: Option<u64>,
    pub run_attempt: Option<u64>,
    pub job: Option<String>,
    pub step_index: Option<u32>,
    pub step_name: Option<String>,
    pub step_key: Option<String>,
    pub cmd: Option<String>,
    pub outcome: String,
    pub exit_code: Option<i32>,
    pub wall_ms: Option<u64>,
    pub saved_ms: Option<u64>,
    pub recorded_ms: Option<u64>,
    pub manifest_id: Option<String>,
    pub inputs: Option<u64>,
    pub source_run: Option<String>,
    pub streak: Option<u32>,
    pub needed: Option<u32>,
    pub git_ref: Option<String>,
    pub sha: Option<String>,
    pub actor: Option<String>,
    pub event: Option<String>,
    pub workflow: Option<String>,
    pub runner: Option<String>,
}

/// One event, as the stream sends it.
#[derive(Clone, Debug)]
pub struct EventRow {
    pub id: i64,
    pub kind: String,
    pub repository: Option<String>,
    pub run_number: Option<u64>,
    pub step_key: Option<String>,
    pub json: Value,
}

impl Activity {
    fn open(path: &Path) -> anyhow::Result<Activity> {
        let db = Connection::open(path)?;
        db.busy_timeout(std::time::Duration::from_secs(30))?;
        db.pragma_update(None, "journal_mode", "WAL")?;
        db.execute_batch(ACTIVITY_SCHEMA)?;
        Ok(Activity { db })
    }

    pub fn insert_step(&self, s: &StepRow) -> anyhow::Result<()> {
        self.db.execute(
            "INSERT INTO steps (at, repository, scope, quod_run, run_id, run_number, run_attempt, job, step_index, step_name, step_key, cmd, outcome, exit_code, wall_ms, saved_ms, recorded_ms, manifest_id, inputs, source_run, streak, needed, git_ref, sha, actor, event, workflow, runner)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,?28)",
            params![
                s.at, s.repository, s.scope, s.quod_run, s.run_id, s.run_number.map(|v| v as i64),
                s.run_attempt.map(|v| v as i64), s.job, s.step_index, s.step_name, s.step_key, s.cmd,
                s.outcome, s.exit_code, s.wall_ms.map(|v| v as i64), s.saved_ms.map(|v| v as i64),
                s.recorded_ms.map(|v| v as i64), s.manifest_id, s.inputs.map(|v| v as i64), s.source_run,
                s.streak, s.needed, s.git_ref, s.sha, s.actor, s.event, s.workflow, s.runner,
            ],
        )?;
        Ok(())
    }

    /// Every step row, oldest first (optionally one repository's).
    pub fn steps(&self, repository: Option<&str>) -> anyhow::Result<Vec<StepRow>> {
        let mut st = self.db.prepare(
            "SELECT at, repository, scope, quod_run, run_id, run_number, run_attempt, job, step_index, step_name, step_key, cmd, outcome, exit_code, wall_ms, saved_ms, recorded_ms, manifest_id, inputs, source_run, streak, needed, git_ref, sha, actor, event, workflow, runner
             FROM steps WHERE (?1 IS NULL OR repository = ?1) ORDER BY id",
        )?;
        let rows = st.query_map(params![repository], |r| {
            let u = |i| r.get::<_, Option<i64>>(i).map(|v| v.map(|v| v as u64));
            Ok(StepRow {
                at: r.get(0)?,
                repository: r.get(1)?,
                scope: r.get(2)?,
                quod_run: r.get(3)?,
                run_id: r.get(4)?,
                run_number: u(5)?,
                run_attempt: u(6)?,
                job: r.get(7)?,
                step_index: r.get(8)?,
                step_name: r.get(9)?,
                step_key: r.get(10)?,
                cmd: r.get(11)?,
                outcome: r.get(12)?,
                exit_code: r.get(13)?,
                wall_ms: u(14)?,
                saved_ms: u(15)?,
                recorded_ms: u(16)?,
                manifest_id: r.get(17)?,
                inputs: u(18)?,
                source_run: r.get(19)?,
                streak: r.get(20)?,
                needed: r.get(21)?,
                git_ref: r.get(22)?,
                sha: r.get(23)?,
                actor: r.get(24)?,
                event: r.get(25)?,
                workflow: r.get(26)?,
                runner: r.get(27)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Appends an event; returns its id.
    pub fn push_event(
        &self,
        kind: &str,
        repository: Option<&str>,
        run_number: Option<u64>,
        step_key: Option<&str>,
        json: &Value,
    ) -> anyhow::Result<i64> {
        self.db.execute(
            "INSERT INTO events (at, type, repository, run_number, step_key, json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                quod_ledger::now_rfc3339(),
                kind,
                repository,
                run_number.map(|v| v as i64),
                step_key,
                json.to_string()
            ],
        )?;
        Ok(self.db.last_insert_rowid())
    }

    /// Events after `after`, oldest first, at most `limit`.
    pub fn events_after(&self, after: i64, limit: usize) -> anyhow::Result<Vec<EventRow>> {
        let mut st = self.db.prepare(
            "SELECT id, type, repository, run_number, step_key, json FROM events WHERE id > ?1 ORDER BY id LIMIT ?2",
        )?;
        let rows = st.query_map(params![after, limit as i64], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<i64>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?;
        let mut out = vec![];
        for row in rows {
            let (id, kind, repository, run, key, json) = row?;
            out.push(EventRow {
                id,
                kind,
                repository,
                run_number: run.map(|v| v as u64),
                step_key: key,
                json: serde_json::from_str(&json)?,
            });
        }
        Ok(out)
    }

    /// The newest event id (0 when none).
    pub fn last_event(&self) -> anyhow::Result<i64> {
        Ok(self
            .db
            .query_row("SELECT max(id) FROM events", [], |r| {
                r.get::<_, Option<i64>>(0)
            })
            .optional()?
            .flatten()
            .unwrap_or(0))
    }
}
