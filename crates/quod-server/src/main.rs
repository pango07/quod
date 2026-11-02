//! `quod-server`: serve quod.v1 (the default), or the operator's helpers.
//!
//! ```text
//! quod-server                 serve, configured from the environment
//! quod-server pubkey          the public key of the configured signing key (QUOD_SERVER_KEY)
//! quod-server hash-token      read a token on stdin, print its BLAKE3 for the config
//! quod-server migrate [--dry-run]   the start-up migration alone (SQLite files, or Postgres)
//! quod-server migrate-sqlite-to-postgres   import the volume's SQLite data into DATABASE_URL and S3_*
//! quod-server healthcheck     exit 0 if this instance answers /healthz (container HEALTHCHECK)
//! ```
//!
//! There is no command that prints a private key: the server generates its
//! signing key on the volume at first boot (`boot`), and only the public
//! key ever leaves it.

use std::io::Read;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "quod=info".into()),
        )
        .json()
        .init();
    let get = |k: &str| std::env::var(k).ok();
    match std::env::args().nth(1).as_deref() {
        None | Some("serve") => {
            if quod_server::boot::euid() == Some(0)
                && get("QUOD_ALLOW_ROOT").as_deref() != Some("1")
            {
                anyhow::bail!(
                    "quod-server: refusing to serve as root (the image's entrypoint drops to the quod user; QUOD_ALLOW_ROOT=1 overrides)"
                );
            }
            let env = quod_server::config::Env::from_env(get)
                .map_err(|e| anyhow::anyhow!("quod-server: {e}"))?;
            let boot =
                quod_server::boot::boot(&env).map_err(|e| anyhow::anyhow!("quod-server: {e}"))?;
            let settings = boot.settings.clone();
            let rt = tokio::runtime::Runtime::new()?;
            let served = rt.block_on(quod_server::serve(settings));
            // In-flight blocking work (a commit's transaction) gets a few
            // seconds; open streams are dropped. A redeploy's new instance
            // waits on the lock for QUOD_LOCK_WAIT_SECS, longer than this.
            rt.shutdown_timeout(std::time::Duration::from_secs(5));
            // The instance lock is held until the server has stopped.
            drop(boot);
            served
        }
        Some("pubkey") => {
            // The public half of the key the server would use: from
            // QUOD_SIGNING_KEY, else the key file (never created here).
            let env = quod_server::config::Env::from_env(get)
                .map_err(|e| anyhow::anyhow!("quod-server: {e}"))?;
            let k = match env
                .signing_key_hex
                .as_ref()
                .map(quod_server::config::Secret::expose)
            {
                Some(h) => quod_wire::signing_key_from_hex(h)?,
                None => {
                    if !env.key_file.is_file() {
                        anyhow::bail!(
                            "no signing key at {} yet: it is generated when the server first starts",
                            env.key_file.display()
                        );
                    }
                    quod_server::boot::load_or_generate(&env.key_file)
                        .map_err(anyhow::Error::msg)?
                        .0
                }
            };
            let pk = k.verifying_key();
            println!("QUOD_SERVER_KEY={}", hex::encode(pk.to_bytes()));
            println!("fingerprint {}", quod_wire::key_fingerprint(&pk));
            Ok(())
        }
        Some("healthcheck") => {
            // For a container HEALTHCHECK: this instance's /healthz.
            let port = get("PORT").unwrap_or_else(|| "8080".into());
            ureq::get(&format!("http://127.0.0.1:{port}/healthz"))
                .timeout(std::time::Duration::from_secs(3))
                .call()?;
            Ok(())
        }
        Some("migrate") => {
            // The start-up migration on its own: what it would do
            // (`--dry-run`, read-only), or do it under the instance lock.
            let dry = match std::env::args().nth(2).as_deref() {
                None => false,
                Some("--dry-run") => true,
                Some(o) => anyhow::bail!("quod-server migrate: unknown argument {o:?} (--dry-run)"),
            };
            let env = quod_server::config::Env::from_env(get)
                .map_err(|e| anyhow::anyhow!("quod-server: {e}"))?;
            if let Some(c) = &env.cloud {
                let pg = quod_server::pg::Pg::open(&c.pg).map_err(anyhow::Error::msg)?;
                let have = pg.recorded_version()?;
                let want = quod_server::pg::PG_SCHEMA_VERSION;
                if dry {
                    println!(
                        "postgres: schema v{have}, this build v{want}: {}",
                        match have.cmp(&want) {
                            std::cmp::Ordering::Less => "would migrate",
                            std::cmp::Ordering::Equal => "current",
                            std::cmp::Ordering::Greater =>
                                "NEWER than this build: the server would refuse to start",
                        }
                    );
                } else {
                    let (from, to) = pg.migrate()?;
                    println!("postgres: schema v{from} -> v{to}");
                }
                return Ok(());
            }
            let store = quod_server::store::Store::new(&env.data_dir);
            if dry {
                let mut too_new = false;
                for p in store.plan() {
                    too_new |= matches!(p.action, quod_server::store::Plan::TooNew(..));
                    println!("{}: {:?}", p.what(), p.action);
                }
                if too_new {
                    println!("a file is newer than this build: the server would refuse to start");
                }
                return Ok(());
            }
            let _lock = quod_server::boot::lock(&env.data_dir, env.lock_wait)
                .map_err(|e| anyhow::anyhow!("quod-server: {e}"))?;
            let m = store.migrate()?;
            println!("{m:?}");
            Ok(())
        }
        Some("migrate-sqlite-to-postgres") => {
            // The volume's SQLite ledgers, team data and blobs into the
            // configured Postgres and bucket (`import`). Holds the
            // instance lock, so it never reads files a SQLite server is
            // writing: stop that server first (on Railway, prefer
            // QUOD_IMPORT_SQLITE=1, which runs this at boot).
            let env = quod_server::config::Env::from_env(get)
                .map_err(|e| anyhow::anyhow!("quod-server: {e}"))?;
            let Some(c) = &env.cloud else {
                anyhow::bail!(
                    "quod-server: set DATABASE_URL and S3_* (the target) to import into them"
                );
            };
            if !env.data_dir.join("teams").is_dir() {
                anyhow::bail!(
                    "quod-server: {} holds no teams/ directory: nothing to import",
                    env.data_dir.display()
                );
            }
            let _lock = quod_server::boot::lock(&env.data_dir, env.lock_wait)
                .map_err(|e| anyhow::anyhow!("quod-server: {e}"))?;
            let cloud =
                quod_server::boot::connect(c).map_err(|e| anyhow::anyhow!("quod-server: {e}"))?;
            let i = quod_server::import::import_sqlite(&env.data_dir, &cloud)?;
            println!("{}", serde_json::to_string_pretty(&i)?);
            Ok(())
        }
        Some("hash-token") => {
            let mut t = String::new();
            std::io::stdin().read_to_string(&mut t)?;
            println!("{}", blake3::hash(t.trim().as_bytes()).to_hex());
            Ok(())
        }
        Some(other) => {
            anyhow::bail!(
                "unknown command {other:?}: serve, pubkey, hash-token, healthcheck, migrate, migrate-sqlite-to-postgres"
            )
        }
    }
}
