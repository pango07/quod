//! The one-shot move of a self-hosted data dir (SQLite and files on the
//! volume) into Postgres and the bucket: `quod-server
//! migrate-sqlite-to-postgres`, or at boot with `QUOD_IMPORT_SQLITE=1`.
//!
//! Per team, in this order:
//!
//! 1. Skipped if Postgres records the team as imported already (`imports`),
//!    refused if Postgres already holds scopes of the team that no import
//!    wrote (a server ran on Postgres first; merging is not attempted).
//! 2. Every blob of `teams/<team>/cas/` goes to the bucket through
//!    `S3Blobs::put_blob_file`, which names the object by the hash of the
//!    bytes it uploads; a file whose bytes do not hash to its name (a
//!    damaged local blob) is reported and stored under its true hash only.
//! 3. Every scope ledger and `team.sqlite`, read-only, copied row by row
//!    in each table's own order (rowid or id), in one Postgres transaction
//!    with the `imports` row: the team is imported whole or not at all.
//!    Audit ids are new (Postgres numbers audits across scopes); a pending
//!    audit's `audit_id` follows its audit. Event ids are new as well, so a
//!    dashboard resuming with an old `Last-Event-ID` starts from the
//!    stream's current end.
//!
//! The SQLite files and the blobs on the volume are only read, never
//! changed or deleted. A ledger at another schema version than this
//! build's is refused (start this build once on SQLite first, which
//! migrates the files, then import).

use std::collections::HashMap;
use std::path::Path;

use quod_cas::{BlobStore, Hash};
use r2d2_postgres::postgres::types::ToSql;
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use crate::s3::S3Blobs;
use crate::store::{ACTIVITY_VERSION, Cloud, RepoKey, ScopeLoc, Store};

/// What an import did.
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub struct Imported {
    /// Teams imported now.
    pub teams: Vec<String>,
    /// Teams Postgres already had from an earlier import.
    pub skipped: Vec<String>,
    pub scopes: usize,
    /// Rows copied, per table.
    pub rows: std::collections::BTreeMap<String, u64>,
    pub blobs: u64,
    pub blob_bytes: u64,
    /// Local blobs whose bytes did not hash to their file name.
    pub damaged_blobs: Vec<String>,
}

/// A target column's type.
#[derive(Clone, Copy)]
enum Ty {
    Text,
    Int,
    Bytes,
    Bool,
}

use Ty::{Bool, Bytes, Int, Text};

fn sql_err(e: impl std::fmt::Display) -> anyhow::Error {
    anyhow::anyhow!("{e}")
}

/// One SQLite value as the Postgres column's type.
fn convert(v: SqlValue, ty: Ty) -> anyhow::Result<Box<dyn ToSql + Sync>> {
    Ok(match (ty, v) {
        (Text, SqlValue::Null) => Box::new(None::<String>),
        (Text, SqlValue::Text(s)) => Box::new(Some(s)),
        (Text, SqlValue::Blob(b)) => Box::new(Some(String::from_utf8(b)?)),
        (Text, SqlValue::Integer(i)) => Box::new(Some(i.to_string())),
        (Int, SqlValue::Null) => Box::new(None::<i64>),
        (Int, SqlValue::Integer(i)) => Box::new(Some(i)),
        (Int, SqlValue::Text(s)) => Box::new(Some(s.parse::<i64>()?)),
        (Bytes, SqlValue::Null) => Box::new(None::<Vec<u8>>),
        (Bytes, SqlValue::Blob(b)) => Box::new(Some(b)),
        (Bytes, SqlValue::Text(s)) => Box::new(Some(s.into_bytes())),
        (Bool, SqlValue::Null) => Box::new(None::<bool>),
        (Bool, SqlValue::Integer(i)) => Box::new(Some(i != 0)),
        (_, v) => anyhow::bail!("unexpected SQLite value {v:?}"),
    })
}

/// Copies `table`'s rows (the columns named, in `order`) into Postgres
/// with `fixed` values first (team, scope). `remap` rewrites one column
/// (an audit id) by a map; `returning` collects the old id column's value
/// to the new id. Returns the rows copied.
#[allow(clippy::too_many_arguments)]
fn copy_table(
    db: &Connection,
    tx: &mut r2d2_postgres::postgres::Transaction<'_>,
    table: &str,
    cols: &[(&str, Ty)],
    order: &str,
    fixed: &[(&str, &(dyn ToSql + Sync))],
    on_conflict: &str,
    ids: Option<(&str, &mut HashMap<i64, i64>)>,
    remap: Option<(&str, &HashMap<i64, i64>)>,
) -> anyhow::Result<u64> {
    let (old_id_col, mut id_map) = match ids {
        Some((c, m)) => (Some(c), Some(m)),
        None => (None, None),
    };
    let mut select: Vec<&str> = cols.iter().map(|(c, _)| *c).collect();
    if let Some(c) = old_id_col {
        select.push(c);
    }
    let names: Vec<&str> = fixed
        .iter()
        .map(|(c, _)| *c)
        .chain(cols.iter().map(|(c, _)| *c))
        .collect();
    let placeholders: Vec<String> = (1..=names.len()).map(|i| format!("${i}")).collect();
    let insert = format!(
        "INSERT INTO {table} ({}) VALUES ({}) {on_conflict}{}",
        names.join(", "),
        placeholders.join(", "),
        if old_id_col.is_some() {
            " RETURNING id"
        } else {
            ""
        }
    );
    let stmt = tx.prepare(&insert).map_err(sql_err)?;
    let mut st = db.prepare(&format!(
        "SELECT {} FROM {table} ORDER BY {order}",
        select.join(", ")
    ))?;
    let mut rows = st.query([])?;
    let mut n = 0;
    while let Some(r) = rows.next()? {
        let mut vals: Vec<Box<dyn ToSql + Sync>> = vec![];
        for (i, (c, ty)) in cols.iter().enumerate() {
            let mut v: SqlValue = r.get(i)?;
            if let (Some((rc, map)), SqlValue::Integer(old)) = (&remap, &v)
                && rc == c
            {
                let new = map.get(old).copied();
                v = new.map_or(SqlValue::Null, SqlValue::Integer);
            }
            vals.push(convert(v, *ty)?);
        }
        let mut params: Vec<&(dyn ToSql + Sync)> = fixed.iter().map(|(_, v)| *v).collect();
        params.extend(vals.iter().map(|b| b.as_ref()));
        match (&mut id_map, old_id_col) {
            (Some(map), Some(_)) => {
                let old: i64 = r.get(cols.len())?;
                let new: i64 = tx
                    .query_one(&stmt, &params)
                    .map_err(sql_err)?
                    .try_get(0)
                    .map_err(sql_err)?;
                map.insert(old, new);
            }
            _ => {
                tx.execute(&stmt, &params).map_err(sql_err)?;
            }
        }
        n += 1;
    }
    Ok(n)
}

fn open_ro(p: &Path) -> anyhow::Result<Connection> {
    Ok(Connection::open_with_flags(
        p,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?)
}

fn version(db: &Connection) -> anyhow::Result<i64> {
    Ok(db.pragma_query_value(None, "user_version", |r| r.get(0))?)
}

/// Uploads a team's local blobs; (count, bytes uploaded, damaged names).
fn import_blobs(cas_dir: &Path, blobs: &S3Blobs) -> anyhow::Result<(u64, u64, Vec<String>)> {
    let (mut n, mut bytes, mut damaged) = (0, 0, vec![]);
    let Ok(shards) = std::fs::read_dir(cas_dir) else {
        return Ok((0, 0, damaged));
    };
    for shard in shards.flatten() {
        if !shard.path().is_dir() {
            continue;
        }
        for f in std::fs::read_dir(shard.path())?.flatten() {
            let name = f.file_name().to_string_lossy().into_owned();
            let Ok(want) = format!("b3:{name}").parse::<Hash>() else {
                continue; // not a blob (a temporary file)
            };
            if !f.path().is_file() {
                continue;
            }
            let (got, sent) = blobs.put_blob_file(&f.path())?;
            if got != want {
                tracing::warn!(target: "quod", blob = %name, actual = %got, "a local blob does not hash to its name; stored under its true hash only");
                damaged.push(name);
            }
            n += 1;
            bytes += sent;
        }
    }
    Ok((n, bytes, damaged))
}

const RUNS: &[(&str, Ty)] = &[
    ("id", Text),
    ("started_at", Text),
    ("commit_sha", Text),
    ("host", Text),
    ("transport", Text),
    ("observer", Text),
    ("step_key", Text),
    ("repo", Text),
    ("repo_path", Text),
    ("mode", Text),
    ("outcome", Text),
    ("manifest_id", Text),
    ("exit_code", Int),
    ("wall_ms", Int),
    ("saved_ms", Int),
    ("suspect", Text),
];
const MANIFESTS: &[(&str, Ty)] = &[
    ("id", Text),
    ("step_key", Text),
    ("run_id", Text),
    ("created_at", Text),
    ("json", Bytes),
    ("root", Text),
];
const RESULTS: &[(&str, Ty)] = &[
    ("id", Text),
    ("step_key", Text),
    ("manifest_id", Text),
    ("run_id", Text),
    ("created_at", Text),
    ("json", Bytes),
];
const BLOBS: &[(&str, Ty)] = &[("hash", Text), ("size", Int), ("path", Text)];
const STEP_KEYS: &[(&str, Ty)] = &[("step_key", Text), ("json", Bytes)];
const TRUST: &[(&str, Ty)] = &[
    ("step_key", Text),
    ("cmd", Text),
    ("state", Text),
    ("eligible", Int),
    ("replays", Int),
    ("audits", Int),
    ("divergences", Int),
    ("quarantined_at", Text),
    ("quarantine_reason", Text),
    ("updated_at", Text),
    ("shadow_from", Int),
    ("volatile", Int),
];
const PROOFS: &[(&str, Ty)] = &[
    ("step_key", Text),
    ("manifest_id", Text),
    ("streak", Int),
    ("fingerprint", Text),
    ("last_run", Text),
    ("updated_at", Text),
];
const AUDITS: &[(&str, Ty)] = &[
    ("step_key", Text),
    ("manifest_id", Text),
    ("at", Text),
    ("phase", Text),
    ("diverged", Bool),
    ("expected_run", Text),
    ("actual_run", Text),
    ("detail", Text),
];
const NODES: &[(&str, Ty)] = &[("id", Text), ("body", Bytes)];
const PENDING: &[(&str, Ty)] = &[
    ("step_key", Text),
    ("manifest_id", Text),
    ("hit_run", Text),
    ("replay_run", Text),
    ("nth", Int),
    ("tree", Text),
    ("request", Bytes),
    ("state", Text),
    ("enqueued_at", Text),
    ("attempts", Int),
    ("claimed_at", Text),
    ("claimed_by", Text),
    ("finished_at", Text),
    ("audit_id", Int),
    ("detail", Text),
    ("repaid", Bool),
];
const NETWORK: &[(&str, Ty)] = &[
    ("step_key", Text),
    ("surface", Text),
    ("reason", Text),
    ("skipped", Int),
    ("updated_at", Text),
    ("inputs", Text),
    ("digest", Text),
];
const STEPS: &[(&str, Ty)] = &[
    ("at", Text),
    ("repository", Text),
    ("scope", Text),
    ("quod_run", Text),
    ("run_id", Text),
    ("run_number", Int),
    ("run_attempt", Int),
    ("job", Text),
    ("step_index", Int),
    ("step_name", Text),
    ("step_key", Text),
    ("cmd", Text),
    ("outcome", Text),
    ("exit_code", Int),
    ("wall_ms", Int),
    ("saved_ms", Int),
    ("recorded_ms", Int),
    ("manifest_id", Text),
    ("inputs", Int),
    ("source_run", Text),
    ("streak", Int),
    ("needed", Int),
    ("git_ref", Text),
    ("sha", Text),
    ("actor", Text),
    ("event", Text),
    ("workflow", Text),
    ("runner", Text),
    ("job_id", Text),
    ("job_name", Text),
];
const EVENTS: &[(&str, Ty)] = &[
    ("at", Text),
    ("type", Text),
    ("repository", Text),
    ("run_number", Int),
    ("step_key", Text),
    ("json", Text),
];
const TRUSTED_WRITES: &[(&str, Ty)] = &[
    ("repository", Text),
    ("git_ref", Text),
    ("protected", Bool),
    ("first_at", Text),
    ("last_at", Text),
    ("writes", Int),
];

/// Imports every team directory of `data_dir` (see the module's doc).
pub fn import_sqlite(data_dir: &Path, cloud: &Cloud) -> anyhow::Result<Imported> {
    let local = Store::new(data_dir);
    let mut out = Imported::default();
    for team in local.teams_on_disk() {
        let mut c = cloud.pg.conn()?;
        let done = c
            .query_opt("SELECT at FROM imports WHERE team = $1", &[&team])
            .map_err(sql_err)?;
        if done.is_some() {
            out.skipped.push(team);
            continue;
        }
        let scopes = local.scope_files(&team);
        let team_db = data_dir.join("teams").join(&team).join("team.sqlite");
        if scopes.is_empty() && !team_db.is_file() {
            continue;
        }
        let existing: i64 = c
            .query_one("SELECT count(*) FROM scopes WHERE team = $1", &[&team])
            .map_err(sql_err)?
            .try_get(0)
            .map_err(sql_err)?;
        if existing > 0 {
            anyhow::bail!(
                "team {team}: Postgres already holds {existing} scope(s) of this team that no import wrote; the import does not merge (nothing was imported for this team)"
            );
        }
        drop(c);
        // Blobs first: a row naming a blob never lands before the blob.
        let blobs = S3Blobs::new(cloud.s3.clone(), &team);
        let (n, bytes, damaged) =
            import_blobs(&data_dir.join("teams").join(&team).join("cas"), &blobs)?;
        out.blobs += n;
        out.blob_bytes += bytes;
        out.damaged_blobs.extend(damaged);

        let mut c = cloud.pg.conn()?;
        let mut tx = c.transaction().map_err(sql_err)?;
        let mut rows: std::collections::BTreeMap<String, u64> = Default::default();
        let mut add = |t: &str, n: u64| *rows.entry(t.to_string()).or_default() += n;
        for s in &scopes {
            let ScopeLoc::File(path) = &s.loc else {
                continue;
            };
            let db = open_ro(path)?;
            let v = version(&db)? as usize;
            if v != quod_ledger::SCHEMA_VERSION {
                anyhow::bail!(
                    "{}: ledger schema v{v}, this build imports v{} only; start this build once with SQLite (it migrates the files), then import",
                    path.display(),
                    quod_ledger::SCHEMA_VERSION
                );
            }
            // The repository key is the directory's name, as the server
            // computed it (`RepoKey::storage_key`).
            let repo_key = path
                .parent()
                .and_then(Path::parent)
                .and_then(Path::file_name)
                .and_then(|n| n.to_str())
                .ok_or_else(|| anyhow::anyhow!("{}: no repository dir", path.display()))?
                .to_string();
            let scope_id: i64 = tx
                .query_one(
                    "INSERT INTO scopes (team, repo_key, repository, scope, created_at) VALUES ($1, $2, $3, $4, $5) RETURNING id",
                    &[&team, &repo_key, &s.repository, &s.scope, &quod_ledger::now_rfc3339()],
                )
                .map_err(sql_err)?
                .try_get(0)
                .map_err(sql_err)?;
            let fixed: [(&str, &(dyn ToSql + Sync)); 2] = [("team", &team), ("scope_id", &scope_id)];
            let mut copy = |table: &str, cols: &[(&str, Ty)], order: &str| -> anyhow::Result<()> {
                let n = copy_table(&db, &mut tx, table, cols, order, &fixed, "", None, None)?;
                add(table, n);
                Ok(())
            };
            copy("runs", RUNS, "rowid")?;
            copy("manifests", MANIFESTS, "rowid")?;
            copy("results", RESULTS, "rowid")?;
            copy("blobs", BLOBS, "rowid")?;
            copy("step_keys", STEP_KEYS, "rowid")?;
            copy("trust", TRUST, "rowid")?;
            copy("proofs", PROOFS, "rowid")?;
            copy("network", NETWORK, "rowid")?;
            let mut audit_ids = HashMap::new();
            let n = copy_table(
                &db,
                &mut tx,
                "audits",
                AUDITS,
                "id",
                &fixed,
                "",
                Some(("id", &mut audit_ids)),
                None,
            )?;
            add("audits", n);
            let mut pending_ids = HashMap::new();
            let n = copy_table(
                &db,
                &mut tx,
                "audits_pending",
                PENDING,
                "id",
                &fixed,
                "",
                Some(("id", &mut pending_ids)),
                Some(("audit_id", &audit_ids)),
            )?;
            add("audits_pending", n);
            let team_only: [(&str, &(dyn ToSql + Sync)); 1] = [("team", &team)];
            let n = copy_table(
                &db,
                &mut tx,
                "manifest_nodes",
                NODES,
                "rowid",
                &team_only,
                "ON CONFLICT (team, id) DO NOTHING",
                None,
                None,
            )?;
            add("manifest_nodes", n);
            out.scopes += 1;
        }
        if team_db.is_file() {
            let db = open_ro(&team_db)?;
            let v = version(&db)?;
            if v != ACTIVITY_VERSION {
                anyhow::bail!(
                    "{}: schema v{v}, this build imports v{ACTIVITY_VERSION} only; start this build once with SQLite, then import",
                    team_db.display()
                );
            }
            let team_only: [(&str, &(dyn ToSql + Sync)); 1] = [("team", &team)];
            for (table, cols, order) in [
                ("steps", STEPS, "id"),
                ("events", EVENTS, "id"),
                ("trusted_writes", TRUSTED_WRITES, "rowid"),
            ] {
                let n = copy_table(&db, &mut tx, table, cols, order, &team_only, "", None, None)?;
                add(table, n);
            }
        }
        let counts = serde_json::to_string(&rows)?;
        tx.execute(
            "INSERT INTO imports (team, at, source, counts) VALUES ($1, $2, $3, $4)",
            &[
                &team,
                &quod_ledger::now_rfc3339(),
                &data_dir.display().to_string(),
                &counts,
            ],
        )
        .map_err(sql_err)?;
        tx.commit().map_err(sql_err)?;
        for (t, n) in rows {
            *out.rows.entry(t).or_default() += n;
        }
        tracing::info!(target: "quod", team = %team, counts = %counts, "team imported from SQLite");
        out.teams.push(team);
    }
    Ok(out)
}

/// Teams with SQLite data on the volume that Postgres has not imported
/// (a boot on Postgres warns about them).
pub fn not_imported(data_dir: &Path, cloud: &Cloud) -> anyhow::Result<Vec<String>> {
    let local = Store::new(data_dir);
    let mut c = cloud.pg.conn()?;
    let mut out = vec![];
    for team in local.teams_on_disk() {
        if local.scope_files(&team).is_empty() {
            continue;
        }
        if c.query_opt("SELECT 1 FROM imports WHERE team = $1", &[&team])
            .map_err(sql_err)?
            .is_none()
        {
            out.push(team);
        }
    }
    Ok(out)
}

/// The repository key a name gets without GitHub's id (for tests).
pub fn repo_key(name: &str) -> String {
    RepoKey::from(name).storage_key()
}
