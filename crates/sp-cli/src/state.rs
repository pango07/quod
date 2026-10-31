use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::sync::Once;

use anyhow::Context;

/// Where Stillpass keeps its files.
pub struct State {
    pub dir: PathBuf,
}

impl State {
    pub fn resolve(flag: Option<&Path>) -> anyhow::Result<State> {
        let dir = match flag {
            Some(d) => d.to_path_buf(),
            None => match std::env::var_os("STILLPASS_HOME") {
                Some(d) => d.into(),
                None => {
                    let home = std::env::var_os("HOME")
                        .context("neither --state, STILLPASS_HOME nor HOME is set")?;
                    Path::new(&home).join(".cache/stillpass")
                }
            },
        };
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("create state dir {}", dir.display()))?;
        let state = State { dir };
        state.init_tracing()?;
        Ok(state)
    }

    pub fn cas_dir(&self) -> PathBuf {
        self.dir.join("cas")
    }

    pub fn ledger_path(&self) -> PathBuf {
        self.dir.join("ledger.sqlite")
    }

    pub fn run_dir(&self, run_id: &str) -> PathBuf {
        self.dir.join("runs").join(run_id)
    }

    pub fn events_path(&self) -> PathBuf {
        self.dir.join("events.jsonl")
    }

    /// The warm Go build cache for steps pinned to `image` (the toolchain):
    /// a persistent host directory the sandbox binds over GOCACHE.
    pub fn warm_gocache_dir(&self, image: &sp_cas::Hash) -> PathBuf {
        self.dir.join("warm").join(image.hex()).join("gocache")
    }

    /// Structured events go to events.jsonl (the bench harness reads them),
    /// never to the step's stdout or stderr.
    fn init_tracing(&self) -> anyhow::Result<()> {
        static ONCE: Once = Once::new();
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.events_path())
            .with_context(|| format!("open {}", self.events_path().display()))?;
        ONCE.call_once(|| {
            tracing_subscriber::fmt()
                .json()
                .flatten_event(true)
                .with_current_span(false)
                .with_writer(std::sync::Mutex::new(file))
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_env("STILLPASS_LOG")
                        .unwrap_or_else(|_| "stillpass=info,warn".into()),
                )
                .init();
        });
        Ok(())
    }
}
