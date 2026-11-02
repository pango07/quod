//! Where a team's data lives, behind one interface for both backends.
//!
//! Self-hosted (SQLite and files): one directory per team, so no query can
//! reach another team's rows or blobs by mistake.
//!
//! ```text
//! <data>/teams/<team>/cas/                         blobs (quod_cas::Cas)
//! <data>/teams/<team>/uploads/                     resumable uploads in progress
//! <data>/teams/<team>/scopes/<repo>/<trust>/ledger.sqlite
//! <data>/teams/<team>/team.sqlite                  CI activity and the event outbox
//! ```
//!
//! `<repo>` is the repository's readable name plus a hash of it (two names
//! never share a directory), or `github-<id>`; its `repository` file holds
//! the name. The scope ledgers are ordinary quod ledgers (quod-ledger's
//! schema: v8 with trust v2, deferred audits' table and the network
//! memory, both unused on a server), the same files a runner keeps
//! locally, migrated by the same code, at start (`Store::migrate`); the
//! server adds nothing to them. What only the server keeps (the
//! dashboard's step rows, the event outbox, trusted writes) is
//! `team.sqlite`, with its own schema version (`ACTIVITY_VERSION`).
//!
//! Quod Cloud (P4): the same rows in Postgres (`pg`), every key and query
//! carrying the team, a scope being a row keyed exactly as the directory
//! above is (team, `<repo>`, scope); blobs in an S3-compatible bucket
//! under `teams/<team>/blobs/` (`s3`). Resumable uploads still stage on the
//! instance's data dir. Handlers see `ScopeLedger`, `TeamBlobs` and
//! `Activity`, which are either backend.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use quod_cas::{BlobStore, Cas, CasError, Hash, Tree};
use quod_ledger::{
    AuditRow, Backlog, Ledger, LedgerError, LedgerStore, Lookup, NewPending, ProofRow, RunFacts,
    RunRow, TrustRow,
};
use quod_manifest::{Manifest, StepResult};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

use crate::pg::{Pg, PgActivity, PgLedger};
use crate::s3::{S3, S3Blobs};

/// A repository as storage keys it: its name, and GitHub's id when known.
#[derive(Clone, Copy, Debug)]
pub struct RepoKey<'a> {
    pub name: &'a str,
    pub github_id: Option<u64>,
}

impl<'a> From<&'a str> for RepoKey<'a> {
    fn from(name: &'a str) -> RepoKey<'a> {
        RepoKey {
            name,
            github_id: None,
        }
    }
}

impl<'a> From<&'a String> for RepoKey<'a> {
    fn from(name: &'a String) -> RepoKey<'a> {
        RepoKey::from(name.as_str())
    }
}

impl RepoKey<'_> {
    /// The repository's key in storage (its directory name self-hosted,
    /// `scopes.repo_key` in Postgres): `github-<repository_id>` when
    /// GitHub's id is known (OIDC), so a rename keeps the ledgers and a new
    /// repository under an old name starts empty; else its readable name
    /// plus a hash.
    pub fn storage_key(&self) -> String {
        if let Some(id) = self.github_id {
            return format!("github-{id}");
        }
        let readable: String = self
            .name
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
        let h = &blake3::hash(self.name.as_bytes()).to_hex()[..12];
        format!("{readable}-{h}")
    }
}

/// The lock that serializes opens of one SQLite file in this process. A
/// file's first open switches it to WAL and migrates it; SQLite answers a
/// concurrent `journal_mode` switch with SQLITE_BUSY at once, without the
/// busy timeout (seen as a 500 under eight racing first commits, S2). One
/// lock per path: opens of different ledgers never wait on each other,
/// and a migration holds only its own file's.
fn path_lock(p: &Path) -> Arc<std::sync::Mutex<()>> {
    static LOCKS: std::sync::Mutex<std::collections::BTreeMap<PathBuf, Arc<std::sync::Mutex<()>>>> =
        std::sync::Mutex::new(std::collections::BTreeMap::new());
    LOCKS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .entry(p.to_path_buf())
        .or_default()
        .clone()
}

/// Opens a scope ledger, one open of that file at a time in this process
/// (`path_lock`). Opens of an existing, current ledger are a read of
/// `user_version`, so the lock costs microseconds.
pub fn open_ledger(p: &Path) -> anyhow::Result<Ledger> {
    let lock = path_lock(p);
    let _one = lock
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Ok(Ledger::open(p)?)
}

/// Quod Cloud's backends: Postgres for the rows, a bucket for the blobs.
pub struct Cloud {
    pub pg: Pg,
    pub s3: S3,
}

#[derive(Clone)]
pub struct Store {
    root: PathBuf,
    cloud: Option<Arc<Cloud>>,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("root", &self.root)
            .field("backend", &self.backend())
            .finish()
    }
}

/// Where one scope ledger is.
#[derive(Clone, Debug)]
pub enum ScopeLoc {
    File(PathBuf),
    Pg(i64),
}

/// One scope ledger of a team.
#[derive(Clone, Debug)]
pub struct ScopeRef {
    pub repository: String,
    pub scope: String,
    pub loc: ScopeLoc,
}

impl Store {
    /// The self-hosted store: SQLite and files under `root`.
    pub fn new(root: impl Into<PathBuf>) -> Store {
        Store {
            root: root.into(),
            cloud: None,
        }
    }

    /// Quod Cloud's store: rows in Postgres, blobs in the bucket; `root`
    /// (the data dir) holds only upload staging.
    pub fn cloud(root: impl Into<PathBuf>, cloud: Arc<Cloud>) -> Store {
        Store {
            root: root.into(),
            cloud: Some(cloud),
        }
    }

    /// `sqlite` or `postgres+s3`.
    pub fn backend(&self) -> &'static str {
        if self.cloud.is_some() {
            "postgres+s3"
        } else {
            "sqlite"
        }
    }

    pub fn cloud_backend(&self) -> Option<&Cloud> {
        self.cloud.as_deref()
    }

    pub fn team_dir(&self, team: &str) -> PathBuf {
        debug_assert!(crate::valid_id(team));
        self.root.join("teams").join(team)
    }

    /// A team's blobs.
    pub fn cas(&self, team: &str) -> anyhow::Result<TeamBlobs> {
        Ok(match &self.cloud {
            Some(c) => TeamBlobs::S3(S3Blobs::new(c.s3.clone(), team)),
            None => TeamBlobs::Local(Cas::open(self.team_dir(team).join("cas"))?),
        })
    }

    /// Where a team's uploads stage (on the data dir in either backend).
    pub fn uploads(&self, team: &str) -> anyhow::Result<PathBuf> {
        let d = self.team_dir(team).join("uploads");
        std::fs::create_dir_all(&d)?;
        Ok(d)
    }

    fn repo_dir(&self, team: &str, repo: RepoKey) -> PathBuf {
        self.team_dir(team).join("scopes").join(repo.storage_key())
    }

    fn ledger_path(&self, team: &str, repo: RepoKey, scope: &str) -> PathBuf {
        debug_assert!(crate::valid_id(scope));
        self.repo_dir(team, repo).join(scope).join("ledger.sqlite")
    }

    pub fn has_ledger<'a>(
        &self,
        team: &str,
        repo: impl Into<RepoKey<'a>>,
        scope: &str,
    ) -> anyhow::Result<bool> {
        let repo = repo.into();
        Ok(match &self.cloud {
            Some(c) => {
                c.pg.scope_id(team, &repo.storage_key(), repo.name, scope, false)?
                    .is_some()
            }
            None => self.ledger_path(team, repo, scope).is_file(),
        })
    }

    /// Opens (creating) a scope's ledger, keeping the repository's newest
    /// name with it.
    pub fn ledger<'a>(
        &self,
        team: &str,
        repo: impl Into<RepoKey<'a>>,
        scope: &str,
    ) -> anyhow::Result<ScopeLedger> {
        let repo = repo.into();
        if let Some(c) = &self.cloud {
            let id =
                c.pg.scope_id(team, &repo.storage_key(), repo.name, scope, true)?
                    .expect("created");
            return Ok(ScopeLedger::Pg(c.pg.ledger(team, id)?));
        }
        let p = self.ledger_path(team, repo, scope);
        let repo_dir = self.repo_dir(team, repo);
        std::fs::create_dir_all(p.parent().unwrap())?;
        let name = repo_dir.join("repository");
        if std::fs::read_to_string(&name).ok().as_deref() != Some(repo.name) {
            std::fs::write(&name, repo.name)?;
        }
        Ok(ScopeLedger::Sqlite(open_ledger(&p)?))
    }

    /// Opens a scope ledger `scopes` listed.
    pub fn open(&self, team: &str, s: &ScopeRef) -> anyhow::Result<ScopeLedger> {
        match (&s.loc, &self.cloud) {
            (ScopeLoc::File(p), _) => Ok(ScopeLedger::Sqlite(open_ledger(p)?)),
            (ScopeLoc::Pg(id), Some(c)) => Ok(ScopeLedger::Pg(c.pg.ledger(team, *id)?)),
            (ScopeLoc::Pg(_), None) => anyhow::bail!("a Postgres scope without Postgres"),
        }
    }

    /// A scope ledger found by the repository's name (as the dashboard
    /// knows repositories), if there is one.
    pub fn ledger_by_name(
        &self,
        team: &str,
        repository: &str,
        scope: &str,
    ) -> anyhow::Result<Option<ScopeLedger>> {
        match self
            .scopes(team)?
            .into_iter()
            .find(|s| s.repository == repository && s.scope == scope)
        {
            Some(s) => Ok(Some(self.open(team, &s)?)),
            None => Ok(None),
        }
    }

    /// Every scope ledger of a team, by repository then scope.
    pub fn scopes(&self, team: &str) -> anyhow::Result<Vec<ScopeRef>> {
        if let Some(c) = &self.cloud {
            return Ok(c
                .pg
                .scopes(team)?
                .into_iter()
                .map(|(id, repository, scope)| ScopeRef {
                    repository,
                    scope,
                    loc: ScopeLoc::Pg(id),
                })
                .collect());
        }
        Ok(self.scope_files(team))
    }

    /// Every SQLite scope ledger of a team on the data dir (whichever
    /// backend serves).
    pub fn scope_files(&self, team: &str) -> Vec<ScopeRef> {
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
                        loc: ScopeLoc::File(path),
                    });
                }
            }
        }
        out.sort_by(|a, b| (&a.repository, &a.scope).cmp(&(&b.repository, &b.scope)));
        out
    }

    /// Every team directory on disk (whether or not the teams document
    /// still names it).
    pub fn teams_on_disk(&self) -> Vec<String> {
        let mut out: Vec<String> = std::fs::read_dir(self.root.join("teams"))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            .filter(|t| crate::valid_id(t))
            .collect();
        out.sort();
        out
    }

    /// Brings every SQLite ledger in the data dir to this binary's
    /// schemas, at start, before anything is served: each scope ledger to
    /// quod-ledger's `SCHEMA_VERSION` (the same migrations a laptop's
    /// ledger runs, each in its own transaction, so a failure leaves that
    /// file as it was) and each `team.sqlite` to `ACTIVITY_VERSION`. A
    /// ledger newer than this binary (a rollback after a newer server ran)
    /// is an error: the server refuses to start rather than answer every
    /// claim of that scope with a 500. Nothing is deleted, and the signing
    /// key is not touched. With Postgres the files are not opened at all
    /// (the importer reads them as they are); Postgres migrates itself
    /// when it connects (`Pg::connect`).
    ///
    /// Only a file newer than this binary stops the boot. Any other
    /// failure (a corrupt or unreadable file, a failed migration) is
    /// logged with the team, scope and path, counted in `failed`, and the
    /// boot goes on: the other scopes serve, and that one answers 500 when
    /// a request opens it (its migration is tried again then).
    pub fn migrate(&self) -> anyhow::Result<Migrated> {
        let mut m = Migrated::default();
        if self.cloud.is_some() {
            return Ok(m);
        }
        let plan = self.plan();
        // A file newer than this binary stops the boot before anything is
        // migrated.
        if let Some(p) = plan.iter().find(|p| matches!(p.action, Plan::TooNew(..))) {
            let Plan::TooNew(v, known) = p.action else {
                unreachable!()
            };
            anyhow::bail!(
                "{}: schema version {v} is newer than this server's {known} (a newer server ran on this volume; roll forward, or restore the volume's backup)",
                p.what()
            );
        }
        for p in plan {
            if p.scope.is_none() {
                m.teams += 1;
            }
            if let Plan::Unreadable(e) = &p.action {
                m.failed += 1;
                tracing::error!(target: "quod", team = %p.team, repository = ?p.repository, scope = ?p.scope, path = %p.path.display(), error = %e, "unreadable; this scope answers 500 until it is repaired");
                continue;
            }
            let opened = match &p.scope {
                None => SqliteActivity::open(&p.path).map(drop),
                Some(_) => open_ledger(&p.path).map(drop),
            };
            match opened {
                Ok(()) => {
                    if p.scope.is_some() {
                        m.ledgers += 1;
                    }
                    if let Plan::Upgrade(from, to) = p.action {
                        m.upgraded += usize::from(p.scope.is_some());
                        tracing::info!(target: "quod", team = %p.team, repository = ?p.repository, scope = ?p.scope, from, to, "migrated");
                    }
                }
                Err(e) => {
                    m.failed += 1;
                    tracing::error!(target: "quod", team = %p.team, repository = ?p.repository, scope = ?p.scope, path = %p.path.display(), error = %format!("{e:#}"), "migration failed; this scope answers 500 until it is repaired");
                }
            }
        }
        Ok(m)
    }

    /// What `migrate` would do to each SQLite file of the data dir, read
    /// without writing (`quod-server migrate --dry-run`).
    pub fn plan(&self) -> Vec<FilePlan> {
        let mut out = vec![];
        for team in self.teams_on_disk() {
            let path = self.team_dir(&team).join("team.sqlite");
            out.push(FilePlan {
                action: Plan::of(&path, ACTIVITY_VERSION as usize),
                team: team.clone(),
                repository: None,
                scope: None,
                path,
            });
            for s in self.scope_files(&team) {
                let ScopeLoc::File(path) = s.loc else {
                    continue;
                };
                out.push(FilePlan {
                    action: Plan::of(&path, quod_ledger::SCHEMA_VERSION),
                    team: team.clone(),
                    repository: Some(s.repository),
                    scope: Some(s.scope),
                    path,
                });
            }
        }
        out
    }

    pub fn activity(&self, team: &str) -> anyhow::Result<Activity> {
        if let Some(c) = &self.cloud {
            return Ok(Activity::Pg(c.pg.activity(team)));
        }
        let d = self.team_dir(team);
        std::fs::create_dir_all(&d)?;
        let p = d.join("team.sqlite");
        Ok(Activity::Sqlite(SqliteActivity::open(&p).map_err(|e| {
            anyhow::anyhow!("team {team} ({}): {e:#}", p.display())
        })?))
    }
}

/// What `Store::migrate` found at start.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Migrated {
    /// Team directories on disk.
    pub teams: usize,
    /// Scope ledgers opened (each now at quod-ledger's `SCHEMA_VERSION`).
    pub ledgers: usize,
    /// Of those, how many were at an older schema before.
    pub upgraded: usize,
    /// Files that could not be read or migrated (logged; their scopes
    /// answer 500, the others serve).
    pub failed: usize,
}

/// What the start-up migration would do to one SQLite file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Plan {
    /// At this binary's version.
    Current(usize),
    /// From, to.
    Upgrade(usize, usize),
    /// No file yet (a team directory without team.sqlite).
    Create,
    /// Newer than this binary (found, known): stops the boot.
    TooNew(usize, usize),
    /// Cannot be opened or read.
    Unreadable(String),
}

impl Plan {
    fn of(path: &Path, known: usize) -> Plan {
        if !path.exists() {
            return Plan::Create;
        }
        let v = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .and_then(|db| db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0)));
        match v {
            Err(e) => Plan::Unreadable(e.to_string()),
            Ok(v) if v < 0 => Plan::Unreadable(format!("user_version {v}")),
            Ok(v) if v as usize > known => Plan::TooNew(v as usize, known),
            Ok(v) if v as usize == known => Plan::Current(known),
            Ok(v) => Plan::Upgrade(v as usize, known),
        }
    }
}

/// One SQLite file of the data dir and its plan.
#[derive(Clone, Debug)]
pub struct FilePlan {
    pub team: String,
    /// The scope ledger's repository and scope; None for team.sqlite.
    pub repository: Option<String>,
    pub scope: Option<String>,
    pub path: PathBuf,
    pub action: Plan,
}

impl FilePlan {
    /// "team acme, acme/app scope trusted (<path>)".
    pub fn what(&self) -> String {
        match (&self.repository, &self.scope) {
            (Some(r), Some(s)) => format!(
                "team {}, {r} scope {s} ({})",
                self.team,
                self.path.display()
            ),
            _ => format!("team {} ({})", self.team, self.path.display()),
        }
    }
}

/// A team's blobs: the local CAS or the bucket.
pub enum TeamBlobs {
    Local(Cas),
    S3(S3Blobs),
}

impl TeamBlobs {
    /// The blob's size, or None when the team has no such blob.
    pub fn size(&self, h: &Hash) -> Result<Option<u64>, CasError> {
        match self {
            TeamBlobs::Local(c) => Ok(std::fs::metadata(c.blob_path(h)).ok().map(|m| m.len())),
            TeamBlobs::S3(s) => s.size(h),
        }
    }
}

impl BlobStore for TeamBlobs {
    fn has_blob(&self, h: &Hash) -> Result<bool, CasError> {
        match self {
            TeamBlobs::Local(c) => c.has_blob(h),
            TeamBlobs::S3(s) => s.has_blob(h),
        }
    }
    fn read_blob(&self, h: &Hash) -> Result<Vec<u8>, CasError> {
        match self {
            TeamBlobs::Local(c) => c.read_blob(h),
            TeamBlobs::S3(s) => s.read_blob(h),
        }
    }
    fn open_blob_reader(&self, h: &Hash) -> Result<Box<dyn std::io::Read + Send>, CasError> {
        match self {
            TeamBlobs::Local(c) => c.open_blob_reader(h),
            TeamBlobs::S3(s) => s.open_blob_reader(h),
        }
    }
    fn put_blob(&self, bytes: &[u8]) -> Result<Hash, CasError> {
        match self {
            TeamBlobs::Local(c) => c.put_blob(bytes),
            TeamBlobs::S3(s) => s.put_blob(bytes),
        }
    }
    fn put_blob_file(&self, path: &Path) -> Result<(Hash, u64), CasError> {
        match self {
            TeamBlobs::Local(c) => c.put_blob_file(path),
            TeamBlobs::S3(s) => s.put_blob_file(path),
        }
    }
}

/// A scope ledger in either backend.
// One per request, moved rarely: the size difference does not matter.
#[allow(clippy::large_enum_variant)]
pub enum ScopeLedger {
    Sqlite(Ledger),
    Pg(PgLedger),
}

macro_rules! each {
    ($s:expr, $l:ident => $e:expr) => {
        match $s {
            ScopeLedger::Sqlite($l) => $e,
            ScopeLedger::Pg($l) => $e,
        }
    };
}

impl ScopeLedger {
    pub fn run(&self, id: &str) -> Result<Option<RunRow>, LedgerError> {
        each!(self, l => l.run(id))
    }
    pub fn trust_rows(&self) -> Result<Vec<TrustRow>, LedgerError> {
        each!(self, l => l.trust_rows())
    }
    pub fn proofs_for_key(&self, step_key: &Hash) -> Result<Vec<ProofRow>, LedgerError> {
        each!(self, l => l.proofs_for_key(step_key))
    }
    pub fn audits(
        &self,
        step_key: Option<&Hash>,
        limit: usize,
    ) -> Result<Vec<AuditRow>, LedgerError> {
        each!(self, l => l.audits(step_key, limit))
    }
    pub fn suspect_runs(&self, step_key: Option<&Hash>) -> Result<Vec<RunRow>, LedgerError> {
        each!(self, l => l.suspect_runs(step_key))
    }
    /// The quod-ledger schema version the rows are at (Postgres mirrors
    /// one, `pg::PG_LEDGER_LEVEL`).
    pub fn schema_version(&self) -> Result<usize, LedgerError> {
        match self {
            ScopeLedger::Sqlite(l) => l.schema_version(),
            ScopeLedger::Pg(_) => Ok(crate::pg::PG_LEDGER_LEVEL),
        }
    }
    /// Deferred-audit rows in any state.
    pub fn pending_total(&self) -> Result<u64, LedgerError> {
        match self {
            ScopeLedger::Sqlite(l) => Ok([
                quod_ledger::PendingState::Pending,
                quod_ledger::PendingState::Running,
                quod_ledger::PendingState::Passed,
                quod_ledger::PendingState::Diverged,
                quod_ledger::PendingState::Lost,
            ]
            .into_iter()
            .map(|s| l.pending_audits(None, s).map(|v| v.len() as u64))
            .sum::<Result<u64, _>>()?),
            ScopeLedger::Pg(l) => l.pending_total(),
        }
    }
}

impl LedgerStore for ScopeLedger {
    fn atomically<T, E: From<LedgerError>>(
        &self,
        f: impl FnOnce(&Self) -> Result<T, E>,
    ) -> Result<T, E> {
        match self {
            ScopeLedger::Sqlite(l) => l.atomically(|_| f(self)),
            ScopeLedger::Pg(l) => l.atomically(|_| f(self)),
        }
    }
    fn find_match(&self, step_key: &Hash, tree: &Tree) -> Result<Lookup, LedgerError> {
        each!(self, l => l.find_match(step_key, tree))
    }
    fn manifest(&self, id: &Hash) -> Result<Option<Manifest>, LedgerError> {
        each!(self, l => l.manifest(id))
    }
    fn latest_result(
        &self,
        step_key: &Hash,
        manifest_id: &Hash,
    ) -> Result<Option<StepResult>, LedgerError> {
        each!(self, l => l.latest_result(step_key, manifest_id))
    }
    fn results_for_run(&self, run_id: &str) -> Result<Vec<StepResult>, LedgerError> {
        each!(self, l => l.results_for_run(run_id))
    }
    fn put_step_key(&self, step_key: &Hash, json: &str) -> Result<(), LedgerError> {
        each!(self, l => l.put_step_key(step_key, json))
    }
    fn record_run_facts(&self, f: &RunFacts) -> Result<(), LedgerError> {
        each!(self, l => l.record_run_facts(f))
    }
    fn record_execution_facts(
        &self,
        run: &RunFacts,
        m: &Manifest,
        r: &StepResult,
    ) -> Result<Hash, LedgerError> {
        each!(self, l => l.record_execution_facts(run, m, r))
    }
    fn trust(&self, step_key: &Hash) -> Result<Option<TrustRow>, LedgerError> {
        each!(self, l => l.trust(step_key))
    }
    fn put_trust(&self, t: &TrustRow) -> Result<(), LedgerError> {
        each!(self, l => l.put_trust(t))
    }
    fn proof(&self, step_key: &Hash, manifest_id: &Hash) -> Result<Option<ProofRow>, LedgerError> {
        each!(self, l => l.proof(step_key, manifest_id))
    }
    fn put_proof(&self, p: &ProofRow) -> Result<(), LedgerError> {
        each!(self, l => l.put_proof(p))
    }
    fn reset_proofs(&self, step_key: &Hash) -> Result<usize, LedgerError> {
        each!(self, l => l.reset_proofs(step_key))
    }
    fn reset_proof(&self, step_key: &Hash, manifest_id: &Hash) -> Result<usize, LedgerError> {
        each!(self, l => l.reset_proof(step_key, manifest_id))
    }
    fn recent_divergences(&self, step_key: &Hash, window: usize) -> Result<u64, LedgerError> {
        each!(self, l => l.recent_divergences(step_key, window))
    }
    fn record_audit(&self, a: &AuditRow) -> Result<i64, LedgerError> {
        each!(self, l => l.record_audit(a))
    }
    fn mark_suspect_replays(&self, step_key: &Hash, why: &str) -> Result<usize, LedgerError> {
        each!(self, l => l.mark_suspect_replays(step_key, why))
    }
    fn backlog(&self, step_key: &Hash, stale_before: &str) -> Result<Backlog, LedgerError> {
        each!(self, l => l.backlog(step_key, stale_before))
    }
    fn enqueue_audit(&self, a: &NewPending) -> Result<i64, LedgerError> {
        each!(self, l => l.enqueue_audit(a))
    }
    fn finish_audit(
        &self,
        id: i64,
        diverged: bool,
        audit_id: i64,
        detail: &str,
    ) -> Result<(), LedgerError> {
        each!(self, l => l.finish_audit(id, diverged, audit_id, detail))
    }
    fn supersede_pending(
        &self,
        step_key: &Hash,
        except: Option<i64>,
        why: &str,
    ) -> Result<usize, LedgerError> {
        each!(self, l => l.supersede_pending(step_key, except, why))
    }
    fn repay_audit(
        &self,
        step_key: &Hash,
        audit_id: i64,
        stale_before: &str,
    ) -> Result<bool, LedgerError> {
        each!(self, l => l.repay_audit(step_key, audit_id, stale_before))
    }
}

/// A team's CI activity (one row per step a runner reported), its event
/// outbox (the dashboard's stream reads it by id, which makes resuming with
/// Last-Event-ID exact) and its trusted writes, in either backend.
pub enum Activity {
    Sqlite(SqliteActivity),
    Pg(PgActivity),
}

impl Activity {
    pub fn insert_step(&self, s: &StepRow) -> anyhow::Result<()> {
        match self {
            Activity::Sqlite(a) => a.insert_step(s),
            Activity::Pg(a) => Ok(a.insert_step(s)?),
        }
    }
    pub fn record_trusted_write(
        &self,
        repository: &str,
        git_ref: &str,
        protected: Option<bool>,
    ) -> anyhow::Result<()> {
        match self {
            Activity::Sqlite(a) => a.record_trusted_write(repository, git_ref, protected),
            Activity::Pg(a) => Ok(a.record_trusted_write(repository, git_ref, protected)?),
        }
    }
    pub fn trusted_writes(&self, repository: Option<&str>) -> anyhow::Result<Vec<TrustedWrite>> {
        match self {
            Activity::Sqlite(a) => a.trusted_writes(repository),
            Activity::Pg(a) => Ok(a.trusted_writes(repository)?),
        }
    }
    pub fn run_number_of(&self, quod_run: &str) -> anyhow::Result<Option<u64>> {
        match self {
            Activity::Sqlite(a) => a.run_number_of(quod_run),
            Activity::Pg(a) => Ok(a.run_number_of(quod_run)?),
        }
    }
    pub fn steps(&self, repository: Option<&str>) -> anyhow::Result<Vec<StepRow>> {
        match self {
            Activity::Sqlite(a) => a.steps(repository),
            Activity::Pg(a) => Ok(a.steps(repository)?),
        }
    }
    pub fn push_event(
        &self,
        kind: &str,
        repository: Option<&str>,
        run_number: Option<u64>,
        step_key: Option<&str>,
        json: &Value,
    ) -> anyhow::Result<i64> {
        match self {
            Activity::Sqlite(a) => a.push_event(kind, repository, run_number, step_key, json),
            Activity::Pg(a) => Ok(a.push_event(kind, repository, run_number, step_key, json)?),
        }
    }
    pub fn events_after(&self, after: i64, limit: usize) -> anyhow::Result<Vec<EventRow>> {
        match self {
            Activity::Sqlite(a) => a.events_after(after, limit),
            Activity::Pg(a) => Ok(a.events_after(after, limit)?),
        }
    }
    pub fn last_event(&self) -> anyhow::Result<i64> {
        match self {
            Activity::Sqlite(a) => a.last_event(),
            Activity::Pg(a) => Ok(a.last_event()?),
        }
    }
}

/// `team.sqlite`: a self-hosted team's activity.
pub struct SqliteActivity {
    db: Connection,
}

/// `team.sqlite`'s schema version: 1, the step rows and the event outbox
/// (unversioned before go-live, `user_version` 0); 2, GitHub's job id and
/// name on step rows, and `trusted_writes`.
pub const ACTIVITY_VERSION: i64 = 2;

const ACTIVITY_V2: &str = "
ALTER TABLE steps ADD COLUMN job_id TEXT;
ALTER TABLE steps ADD COLUMN job_name TEXT;
CREATE TABLE IF NOT EXISTS trusted_writes (
  repository TEXT NOT NULL, git_ref TEXT NOT NULL, protected INTEGER, first_at TEXT NOT NULL,
  last_at TEXT NOT NULL, writes INTEGER NOT NULL, PRIMARY KEY (repository, git_ref));
CREATE INDEX IF NOT EXISTS steps_by_quod_run ON steps (quod_run);
";

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
    /// GitHub's job id (`job.check_run_id`), when the runner reported it.
    pub job_id: Option<String>,
    /// GitHub's display name of the job (matrix legs differ).
    pub job_name: Option<String>,
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

/// Trusted results written from one ref of a repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustedWrite {
    pub repository: String,
    pub git_ref: String,
    /// GitHub's `ref_protected` claim at the newest write; `None` when the
    /// writer was not an OIDC job (unknown).
    pub protected: Option<bool>,
    pub first_at: String,
    pub last_at: String,
    pub writes: u64,
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

impl SqliteActivity {
    /// Opens (creating and migrating) a team.sqlite, one open of that file
    /// at a time in this process: its first open switches it to WAL, which
    /// races as a scope ledger's does (`path_lock`).
    fn open(path: &Path) -> anyhow::Result<SqliteActivity> {
        let lock = path_lock(path);
        let _one = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let db = Connection::open(path)?;
        db.busy_timeout(std::time::Duration::from_secs(30))?;
        db.pragma_update(None, "journal_mode", "WAL")?;
        let current: i64 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if current == ACTIVITY_VERSION {
            return Ok(SqliteActivity { db });
        }
        // One migration at a time across connections: BEGIN IMMEDIATE.
        db.execute_batch("BEGIN IMMEDIATE")?;
        let migrated = (|| -> anyhow::Result<()> {
            let v: i64 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
            if v > ACTIVITY_VERSION {
                anyhow::bail!(
                    "{} has schema version {v}, newer than this server's {ACTIVITY_VERSION}",
                    path.display()
                );
            }
            db.execute_batch(ACTIVITY_SCHEMA)?;
            if v < 2 {
                db.execute_batch(ACTIVITY_V2)?;
            }
            db.pragma_update(None, "user_version", ACTIVITY_VERSION)?;
            Ok(())
        })();
        match migrated {
            Ok(()) => db.execute_batch("COMMIT")?,
            Err(e) => {
                let _ = db.execute_batch("ROLLBACK");
                return Err(e);
            }
        }
        Ok(SqliteActivity { db })
    }

    pub fn insert_step(&self, s: &StepRow) -> anyhow::Result<()> {
        self.db.execute(
            "INSERT INTO steps (at, repository, scope, quod_run, run_id, run_number, run_attempt, job, step_index, step_name, step_key, cmd, outcome, exit_code, wall_ms, saved_ms, recorded_ms, manifest_id, inputs, source_run, streak, needed, git_ref, sha, actor, event, workflow, runner, job_id, job_name)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,?28,?29,?30)",
            params![
                s.at, s.repository, s.scope, s.quod_run, s.run_id, s.run_number.map(|v| v as i64),
                s.run_attempt.map(|v| v as i64), s.job, s.step_index, s.step_name, s.step_key, s.cmd,
                s.outcome, s.exit_code, s.wall_ms.map(|v| v as i64), s.saved_ms.map(|v| v as i64),
                s.recorded_ms.map(|v| v as i64), s.manifest_id, s.inputs.map(|v| v as i64), s.source_run,
                s.streak, s.needed, s.git_ref, s.sha, s.actor, s.event, s.workflow, s.runner,
                s.job_id, s.job_name,
            ],
        )?;
        Ok(())
    }

    /// Notes a write into a repository's trusted scope from `git_ref`.
    pub fn record_trusted_write(
        &self,
        repository: &str,
        git_ref: &str,
        protected: Option<bool>,
    ) -> anyhow::Result<()> {
        let now = quod_ledger::now_rfc3339();
        self.db.execute(
            "INSERT INTO trusted_writes (repository, git_ref, protected, first_at, last_at, writes) VALUES (?1, ?2, ?3, ?4, ?4, 1)
             ON CONFLICT (repository, git_ref) DO UPDATE SET protected = excluded.protected, last_at = excluded.last_at, writes = writes + 1",
            params![repository, git_ref, protected, now],
        )?;
        Ok(())
    }

    /// Trusted writes, by repository then ref.
    pub fn trusted_writes(&self, repository: Option<&str>) -> anyhow::Result<Vec<TrustedWrite>> {
        let mut st = self.db.prepare(
            "SELECT repository, git_ref, protected, first_at, last_at, writes FROM trusted_writes
             WHERE (?1 IS NULL OR repository = ?1) ORDER BY repository, git_ref",
        )?;
        let rows = st.query_map(params![repository], |r| {
            Ok(TrustedWrite {
                repository: r.get(0)?,
                git_ref: r.get(1)?,
                protected: r.get(2)?,
                first_at: r.get(3)?,
                last_at: r.get(4)?,
                writes: r.get::<_, i64>(5)? as u64,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The CI run number of the newest step row of a quod run.
    pub fn run_number_of(&self, quod_run: &str) -> anyhow::Result<Option<u64>> {
        Ok(self
            .db
            .query_row(
                "SELECT run_number FROM steps WHERE quod_run = ?1 ORDER BY id DESC LIMIT 1",
                params![quod_run],
                |r| r.get::<_, Option<i64>>(0),
            )
            .optional()?
            .flatten()
            .map(|v| v as u64))
    }

    /// Every step row, oldest first (optionally one repository's).
    pub fn steps(&self, repository: Option<&str>) -> anyhow::Result<Vec<StepRow>> {
        let mut st = self.db.prepare(
            "SELECT at, repository, scope, quod_run, run_id, run_number, run_attempt, job, step_index, step_name, step_key, cmd, outcome, exit_code, wall_ms, saved_ms, recorded_ms, manifest_id, inputs, source_run, streak, needed, git_ref, sha, actor, event, workflow, runner, job_id, job_name
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
                job_id: r.get(28)?,
                job_name: r.get(29)?,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_team_db_from_before_go_live_migrates() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("team.sqlite");
        {
            // As exp/cloud-ledger left it: the v1 tables, no user_version.
            let db = Connection::open(&p).unwrap();
            db.execute_batch(ACTIVITY_SCHEMA).unwrap();
            db.execute(
                "INSERT INTO steps (at, repository, scope, quod_run, outcome) VALUES ('2026-11-01T00:00:00.000Z', 'acme/app', 'trusted', 'R1', 'miss')",
                [],
            )
            .unwrap();
        }
        let a = SqliteActivity::open(&p).unwrap();
        let rows = a.steps(None).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].job_id.is_none());
        a.record_trusted_write("acme/app", "refs/heads/main", None)
            .unwrap();
        a.record_trusted_write("acme/app", "refs/heads/main", Some(false))
            .unwrap();
        let w = a.trusted_writes(Some("acme/app")).unwrap();
        assert_eq!((w.len(), w[0].writes, w[0].protected), (1, 2, Some(false)));
        drop(a);
        // Opening again is a no-op; a newer schema is refused.
        SqliteActivity::open(&p).unwrap();
        Connection::open(&p)
            .unwrap()
            .pragma_update(None, "user_version", ACTIVITY_VERSION + 1)
            .unwrap();
        assert!(SqliteActivity::open(&p).is_err());
    }
}
