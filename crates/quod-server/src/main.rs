//! `quod-server`: serve quod.v1 (the default), or the operator's helpers.
//!
//! ```text
//! quod-server                 serve, configured from the environment
//! quod-server keygen          a new signing key seed (QUOD_SIGNING_KEY) and its public key
//! quod-server pubkey          the public key of QUOD_SIGNING_KEY (runners' QUOD_SERVER_KEY)
//! quod-server hash-token      read a token on stdin, print its BLAKE3 for the config
//! quod-server healthcheck     exit 0 if this instance answers /healthz (container HEALTHCHECK)
//! ```

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
            let settings = quod_server::config::Settings::from_env(get)
                .map_err(|e| anyhow::anyhow!("quod-server: {e}"))?;
            tokio::runtime::Runtime::new()?.block_on(quod_server::serve(settings))
        }
        Some("keygen") => {
            let k = quod_server::config::generate_key().map_err(anyhow::Error::msg)?;
            println!("QUOD_SIGNING_KEY={}", hex::encode(k.to_bytes()));
            println!(
                "QUOD_SERVER_KEY={}",
                hex::encode(k.verifying_key().to_bytes())
            );
            Ok(())
        }
        Some("pubkey") => {
            let k = quod_wire::signing_key_from_hex(
                &get("QUOD_SIGNING_KEY")
                    .ok_or_else(|| anyhow::anyhow!("QUOD_SIGNING_KEY is not set"))?,
            )?;
            println!("{}", hex::encode(k.verifying_key().to_bytes()));
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
            anyhow::bail!(
                "unknown command {other:?}: serve, keygen, pubkey, hash-token, healthcheck"
            )
        }
    }
}
