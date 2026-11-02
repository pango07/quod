//! Start-up checks and state on the data dir, before anything listens.
//!
//! - The data dir must be writable and known to persist: a Railway volume
//!   that covers it (`RAILWAY_VOLUME_MOUNT_PATH`), or the operator's
//!   attestation `QUOD_DATA_DIR_PERSISTENT=1`. Results on a disk that a
//!   redeploy wipes would be lost silently; the server refuses instead.
//! - One instance per data dir: an exclusive lock on
//!   `$QUOD_DATA_DIR/quod-server.lock`, held for the life of the process.
//!   SQLite on a volume is single-writer by design, and the trust
//!   transitions rely on one serializing writer per ledger. A second
//!   instance waits `QUOD_LOCK_WAIT_SECS` (a redeploy's overlap), then
//!   refuses to start. With Postgres the ledger needs no such lock (its
//!   writers serialize on Postgres advisory locks, across instances); the
//!   lock stays because the data dir still holds per-instance state that
//!   two processes must not share: the key file's first generation and
//!   the resumable upload staging files.
//! - With Postgres and S3 (`DATABASE_URL` and `S3_*`): both are connected
//!   and checked (`connect`), Postgres migrated, and with
//!   `QUOD_IMPORT_SQLITE=1` the volume's SQLite data imported, all before
//!   listening.
//! - The signing key: `QUOD_SIGNING_KEY` if the environment has one, else
//!   the key file (`QUOD_SIGNING_KEY_FILE`, default
//!   `$QUOD_DATA_DIR/signing.ed25519`), generated on first boot from the
//!   OS's randomness and written 0600 atomically. Only the public key's
//!   fingerprint is logged; the private key is never logged or served.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use ed25519_dalek::SigningKey;

use crate::config::{Env, Secret, Settings, fingerprint, generate_key};

/// The lock file's name inside the data dir.
pub const LOCK_FILE: &str = "quod-server.lock";

/// A started server's state: the settings, and the instance lock, which
/// must live as long as the server.
pub struct Boot {
    pub settings: Settings,
    pub lock: InstanceLock,
    /// Why the data dir counts as persistent.
    pub persistence: String,
    /// Whether the signing key was generated on this boot.
    pub key_generated: bool,
    /// The ledgers brought to this binary's schemas before serving.
    pub migrated: crate::store::Migrated,
    /// What `QUOD_IMPORT_SQLITE=1` imported at this boot.
    pub imported: Option<crate::import::Imported>,
}

/// Connects Quod Cloud's backends: Postgres (pooled, migrated under its
/// advisory lock) and the bucket, which must take and give back a blob
/// (`probe`): a server that cannot store results refuses to start rather
/// than answer every commit with a 500.
pub fn connect(c: &crate::config::CloudEnv) -> Result<crate::store::Cloud, String> {
    let pg = crate::pg::Pg::connect(&c.pg)?;
    let v = pg.schema_version().map_err(|e| format!("postgres: {e}"))?;
    let s3 = crate::s3::S3::new(c.s3.clone())?;
    let probe = crate::s3::S3Blobs::new(s3.clone(), "quod-probe");
    let bytes = b"quod-server: the bucket takes and gives back a blob";
    let back = quod_cas::BlobStore::put_blob(&probe, bytes)
        .and_then(|h| quod_cas::BlobStore::read_blob(&probe, &h))
        .map_err(|e| format!("object store {} bucket {}: {e}", c.s3.endpoint, c.s3.bucket))?;
    if back != bytes {
        return Err("object store: the probe blob came back different".into());
    }
    tracing::info!(target: "quod", pg_schema = v, pool = c.pg.pool_size, endpoint = %c.s3.endpoint, bucket = %c.s3.bucket, path_style = c.s3.path_style, "postgres and object store ready");
    Ok(crate::store::Cloud { pg, s3 })
}

/// The exclusive lock on the data dir; released when dropped (or when the
/// process ends, however it ends).
pub struct InstanceLock {
    _file: File,
    pub path: PathBuf,
}

/// This process's effective uid, from `/proc/self/status` (Linux, where
/// the server runs); `None` elsewhere.
pub fn euid() -> Option<u32> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = s.lines().find(|l| l.starts_with("Uid:"))?;
    line.split_whitespace().nth(2)?.parse().ok()
}

/// Every check, then the settings.
pub fn boot(env: &Env) -> Result<Boot, String> {
    let dir = &env.data_dir;
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("QUOD_DATA_DIR {}: cannot create it: {e}", dir.display()))?;
    check_writable(dir)?;
    let persistence = persistence(env)?;
    let lock = lock(dir, env.lock_wait)?;
    let (signing_key, key_generated) = signing_key(env)?;
    let doc = env.document()?;
    let (cloud, imported) = match &env.cloud {
        None => (None, None),
        Some(c) => {
            let cloud = Arc::new(connect(c)?);
            let imported = if env.import_sqlite {
                let i = crate::import::import_sqlite(dir, &cloud)
                    .map_err(|e| format!("import of the volume's SQLite data: {e:#}"))?;
                tracing::info!(target: "quod", teams = ?i.teams, skipped = ?i.skipped, scopes = i.scopes, blobs = i.blobs, rows = ?i.rows, "SQLite data imported");
                Some(i)
            } else {
                None
            };
            match crate::import::not_imported(dir, &cloud) {
                Ok(t) if !t.is_empty() => {
                    tracing::warn!(target: "quod", teams = ?t, "the volume holds SQLite ledgers of teams that Postgres has not imported; they are not served (QUOD_IMPORT_SQLITE=1 imports them at the next boot)");
                }
                Ok(_) => {}
                Err(e) => return Err(format!("postgres: {e:#}")),
            }
            (Some(cloud), imported)
        }
    };
    let store = match &cloud {
        Some(c) => crate::store::Store::cloud(dir, c.clone()),
        None => crate::store::Store::new(dir),
    };
    // Under the lock, so no older instance still serves these files, and
    // before listening, so no claim meets a ledger mid-migration. With
    // Postgres the files are left as they are (Postgres migrated in
    // `connect`).
    let migrated = if env.skip_migration {
        tracing::warn!(target: "quod", "QUOD_SKIP_MIGRATION=1: no start-up migration pass; each SQLite ledger migrates when a request first opens it");
        crate::store::Migrated::default()
    } else {
        store
            .migrate()
            .map_err(|e| format!("data dir migration: {e:#}"))?
    };
    if migrated.failed > 0 {
        tracing::error!(target: "quod", failed = migrated.failed, "some SQLite files could not be read or migrated (logged above); their scopes answer 500, the rest serve");
    }
    tracing::info!(
        target: "quod",
        backend = store.backend(),
        teams = migrated.teams,
        scope_ledgers = migrated.ledgers,
        upgraded = migrated.upgraded,
        ledger_schema = quod_ledger::SCHEMA_VERSION,
        team_schema = crate::store::ACTIVITY_VERSION,
        pg_schema = crate::pg::PG_SCHEMA_VERSION,
        "ledgers ready"
    );
    let pk = signing_key.verifying_key();
    tracing::info!(
        target: "quod",
        fingerprint = %fingerprint(&pk),
        generated = key_generated,
        source = if env.signing_key_hex.is_some() { "QUOD_SIGNING_KEY" } else { "key file" },
        "signing key ready; clients pin its public key from GET /v1/keys"
    );
    tracing::info!(target: "quod", data_dir = %dir.display(), persistence = %persistence, teams = doc.teams.len(), "data dir ready");
    Ok(Boot {
        settings: Settings {
            bind: env.bind.clone(),
            port: env.port,
            data_dir: dir.clone(),
            signing_key,
            doc,
            workos_api_key: env.workos_api_key.as_ref().map(|s| s.expose().to_string()),
            sse_hold: env.sse_hold,
            max_blob: env.max_blob,
            max_json: env.max_json,
            cloud,
        },
        lock,
        persistence,
        key_generated,
        migrated,
        imported,
    })
}

/// A file can be created, written, synced and removed in `dir`.
fn check_writable(dir: &Path) -> Result<(), String> {
    let probe = dir.join(format!(".quod-write-probe-{}", std::process::id()));
    let r = (|| -> std::io::Result<()> {
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&probe)?;
        f.write_all(b"quod")?;
        f.sync_all()?;
        std::fs::remove_file(&probe)
    })();
    r.map_err(|e| {
        format!(
            "QUOD_DATA_DIR {} is not writable by this process (uid {}): {e}",
            dir.display(),
            euid().map_or("unknown".into(), |u| u.to_string())
        )
    })
}

/// Why the data dir persists, or why the server refuses to trust that it
/// does.
pub fn persistence(env: &Env) -> Result<String, String> {
    let dir = &env.data_dir;
    let covered = env
        .railway_volume
        .as_ref()
        .is_some_and(|v| dir.starts_with(v));
    if env.on_railway {
        return match (&env.railway_volume, covered) {
            (Some(v), true) => Ok(format!("Railway volume mounted at {}", v.display())),
            (Some(v), false) => Err(format!(
                "QUOD_DATA_DIR {} is not inside this service's Railway volume (mounted at {}): it would be lost on redeploy. Set QUOD_DATA_DIR to the volume's mount path.",
                dir.display(),
                v.display()
            )),
            (None, _) if env.persistent_attested => {
                tracing::warn!(target: "quod", "on Railway with no volume reported (RAILWAY_VOLUME_MOUNT_PATH unset); trusting QUOD_DATA_DIR_PERSISTENT=1");
                Ok("QUOD_DATA_DIR_PERSISTENT=1 (Railway reports no volume)".into())
            }
            (None, _) => Err(format!(
                "this Railway service has no volume (RAILWAY_VOLUME_MOUNT_PATH is unset), so QUOD_DATA_DIR {} would be lost on redeploy. Attach a volume mounted at {}.",
                dir.display(),
                dir.display()
            )),
        };
    }
    if covered {
        return Ok(format!(
            "volume mounted at {}",
            env.railway_volume.as_ref().unwrap().display()
        ));
    }
    if env.persistent_attested {
        return Ok("QUOD_DATA_DIR_PERSISTENT=1".into());
    }
    if env.dev {
        tracing::warn!(target: "quod", data_dir = %dir.display(), "QUOD_DEV=1: the data dir is not checked for persistence");
        return Ok("QUOD_DEV=1 (not checked)".into());
    }
    Err(format!(
        "QUOD_DATA_DIR {} is not known to persist across restarts. Mount a persistent volume there and set QUOD_DATA_DIR_PERSISTENT=1 (on Railway, attaching a volume at that path is enough).",
        dir.display()
    ))
}

/// Takes the data dir's instance lock, waiting up to `wait` for another
/// instance (a redeploy's overlap) to let go.
pub fn lock(dir: &Path, wait: std::time::Duration) -> Result<InstanceLock, String> {
    let path = dir.join(LOCK_FILE);
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(&path)
        .map_err(|e| format!("lock file {}: {e}", path.display()))?;
    let start = Instant::now();
    let mut said = false;
    loop {
        match file.try_lock() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) => {
                if start.elapsed() >= wait {
                    return Err(format!(
                        "another quod-server holds {} (this server is single-replica: one instance per data dir; set numReplicas to 1)",
                        path.display()
                    ));
                }
                if !said {
                    tracing::warn!(target: "quod", lock = %path.display(), "another instance holds the data dir; waiting for it to stop");
                    said = true;
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            Err(std::fs::TryLockError::Error(e)) => {
                return Err(format!("lock file {}: {e}", path.display()));
            }
        }
    }
    // Who holds it, for an operator looking at the volume.
    let _ = file.set_len(0);
    let _ = writeln!(
        file,
        "pid {} since {}",
        std::process::id(),
        quod_ledger::now_rfc3339()
    );
    Ok(InstanceLock { _file: file, path })
}

/// The signing key, and whether it was generated now.
fn signing_key(env: &Env) -> Result<(SigningKey, bool), String> {
    if let Some(hex) = env.signing_key_hex.as_ref().map(Secret::expose) {
        let k =
            quod_wire::signing_key_from_hex(hex).map_err(|e| format!("QUOD_SIGNING_KEY: {e}"))?;
        return Ok((k, false));
    }
    load_or_generate(&env.key_file)
}

/// Reads the key file, or creates it: a fresh seed from the OS, written
/// 0600 to a temporary file in the same directory, synced, then linked to
/// its name (which fails rather than replace a key another process wrote
/// meanwhile), and the directory synced.
pub fn load_or_generate(path: &Path) -> Result<(SigningKey, bool), String> {
    let show = path.display();
    match std::fs::metadata(path) {
        Ok(meta) => {
            let mode = meta.permissions().mode() & 0o777;
            if mode & 0o077 != 0 {
                return Err(format!(
                    "signing key {show} is readable by others (mode {mode:o}); it must be 0600"
                ));
            }
            let hex =
                std::fs::read_to_string(path).map_err(|e| format!("signing key {show}: {e}"))?;
            let k = quod_wire::signing_key_from_hex(hex.trim())
                .map_err(|e| format!("signing key {show}: {e}"))?;
            Ok((k, false))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let dir = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("signing key dir {}: {e}", dir.display()))?;
            let k = generate_key()?;
            let tmp = dir.join(format!(
                ".{}.tmp-{}",
                path.file_name().and_then(|n| n.to_str()).unwrap_or("key"),
                std::process::id()
            ));
            let _ = std::fs::remove_file(&tmp);
            let written = (|| -> std::io::Result<()> {
                let mut f = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&tmp)?;
                f.write_all(hex::encode(k.to_bytes()).as_bytes())?;
                f.write_all(b"\n")?;
                f.sync_all()?;
                std::fs::hard_link(&tmp, path)?;
                File::open(dir)?.sync_all()
            })();
            let _ = std::fs::remove_file(&tmp);
            match written {
                Ok(()) => Ok((k, true)),
                // Another process created it between the check and the link.
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => load_or_generate(path),
                Err(e) => Err(format!("signing key {show}: cannot write it: {e}")),
            }
        }
        Err(e) => Err(format!("signing key {show}: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Env;

    fn env_for(dir: &Path, extra: &[(&str, &str)]) -> Env {
        let d = dir.to_str().unwrap().to_string();
        let pairs: Vec<(String, String)> = [("QUOD_DATA_DIR".to_string(), d)]
            .into_iter()
            .chain(extra.iter().map(|(k, v)| (k.to_string(), v.to_string())))
            .collect();
        Env::from_env(|k| pairs.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())).unwrap()
    }

    #[test]
    fn a_data_dir_not_known_to_persist_is_refused() {
        let t = tempfile::tempdir().unwrap();
        let e = boot(&env_for(t.path(), &[])).err().unwrap();
        assert!(e.contains("QUOD_DATA_DIR_PERSISTENT=1"), "{e}");
        // Attested: accepted.
        assert!(boot(&env_for(t.path(), &[("QUOD_DATA_DIR_PERSISTENT", "1")])).is_ok());
    }

    #[test]
    fn railway_needs_the_data_dir_inside_its_volume() {
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("data");
        let d = data.to_str().unwrap();
        let on = [("RAILWAY_PROJECT_ID", "p")];
        // No volume: refused, even attested only with a warning.
        let e = persistence(&env_for(&data, &on)).unwrap_err();
        assert!(e.contains("no volume"), "{e}");
        // A volume elsewhere: refused whatever the attestation says.
        let e = persistence(&env_for(
            &data,
            &[
                ("RAILWAY_PROJECT_ID", "p"),
                ("RAILWAY_VOLUME_MOUNT_PATH", "/elsewhere"),
                ("QUOD_DATA_DIR_PERSISTENT", "1"),
            ],
        ))
        .unwrap_err();
        assert!(e.contains("not inside"), "{e}");
        // The volume at the data dir: accepted without an attestation.
        let ok = persistence(&env_for(
            &data,
            &[
                ("RAILWAY_PROJECT_ID", "p"),
                ("RAILWAY_VOLUME_MOUNT_PATH", d),
            ],
        ))
        .unwrap();
        assert!(ok.contains("Railway volume"), "{ok}");
    }

    #[test]
    fn an_unwritable_data_dir_is_refused() {
        if euid() == Some(0) {
            return; // root writes anywhere
        }
        let t = tempfile::tempdir().unwrap();
        let ro = t.path().join("ro");
        std::fs::create_dir(&ro).unwrap();
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o500)).unwrap();
        let e = boot(&env_for(&ro, &[("QUOD_DATA_DIR_PERSISTENT", "1")]))
            .err()
            .unwrap();
        assert!(e.contains("not writable"), "{e}");
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[test]
    fn a_second_instance_is_refused() {
        let t = tempfile::tempdir().unwrap();
        let first = lock(t.path(), std::time::Duration::ZERO).unwrap();
        let e = lock(t.path(), std::time::Duration::from_millis(600))
            .err()
            .unwrap();
        assert!(e.contains("single-replica"), "{e}");
        drop(first);
        assert!(lock(t.path(), std::time::Duration::ZERO).is_ok());
    }

    #[test]
    fn the_key_is_generated_once_0600_and_reused() {
        let t = tempfile::tempdir().unwrap();
        let env = env_for(t.path(), &[("QUOD_DATA_DIR_PERSISTENT", "1")]);
        let b1 = boot(&env).unwrap();
        assert!(b1.key_generated);
        let path = t.path().join("signing.ed25519");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let pk1 = b1.settings.signing_key.verifying_key();
        drop(b1);
        let b2 = boot(&env).unwrap();
        assert!(!b2.key_generated);
        assert_eq!(b2.settings.signing_key.verifying_key(), pk1);
        // No temporary file is left behind.
        let left: Vec<_> = std::fs::read_dir(t.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".tmp-"))
            .collect();
        assert!(left.is_empty(), "{left:?}");
        drop(b2);
        // A key file others can read is refused.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(boot(&env).err().unwrap().contains("0600"));
    }

    #[test]
    fn the_teams_file_on_the_volume_is_read() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(
            t.path().join("teams.json"),
            r#"{"teams":[{"slug":"quodci","github_owner_ids":[104618549]}]}"#,
        )
        .unwrap();
        let b = boot(&env_for(t.path(), &[("QUOD_DATA_DIR_PERSISTENT", "1")])).unwrap();
        assert_eq!(b.settings.doc.teams[0].slug, "quodci");
    }
}
