//! `quod-server`: serve quod.v1 (the default), or the operator's helpers.
//!
//! ```text
//! quod-server                 serve, configured from the environment
//! quod-server pubkey          the public key of the configured signing key (QUOD_SERVER_KEY)
//! quod-server hash-token      read a token on stdin, print its BLAKE3 for the config
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
        Some("hash-token") => {
            let mut t = String::new();
            std::io::stdin().read_to_string(&mut t)?;
            println!("{}", blake3::hash(t.trim().as_bytes()).to_hex());
            Ok(())
        }
        Some(other) => {
            anyhow::bail!("unknown command {other:?}: serve, pubkey, hash-token, healthcheck")
        }
    }
}
