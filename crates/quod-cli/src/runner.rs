//! `quod runner`: a warm pool of official actions/runner processes wired
//! to the quod hooks, and their deregistration.
//!
//! `pool --size N` installs N runners under `<state>/runners/<i>` (the pinned
//! release from versions.toml, downloaded once), registers each with the
//! label `quod`, pre-unpacks the images named with `--image`, and keeps
//! `run.sh` alive for each until the pool is stopped.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use clap::{Args, Subcommand};
use quod_hooks::rootfs::{OciRootfs, RootfsProvider};

use crate::state::State;

const VERSIONS_TOML: &str = include_str!("../../../versions.toml");
const HOOK_SHIM: &str = include_str!("../../../scripts/runner/hooks/index.js");

#[derive(Subcommand)]
pub enum RunnerCmd {
    /// Keep N registered runners warm.
    Pool(PoolArgs),
    /// Remove this pool's runners from the repository.
    Deregister(DeregisterArgs),
}

#[derive(Args)]
pub struct PoolArgs {
    #[arg(long, default_value_t = 1)]
    size: u32,
    /// owner/repo to register with.
    #[arg(long)]
    repo: String,
    /// A registration token (repos/{repo}/actions/runners/registration-token).
    /// This puts the token on quod's own argv, where `ps` can read it
    /// for the life of the process (the token still expires in an hour, and
    /// config.sh itself takes it on argv regardless); prefer --token-cmd,
    /// which keeps it off quod's argv.
    #[arg(long, conflicts_with = "token_cmd")]
    token: Option<String>,
    /// A command printing a registration token, run once per runner that
    /// needs registering (tokens expire after an hour). Keeps the token off
    /// quod's argv; it still reaches config.sh's argv, briefly.
    #[arg(long)]
    token_cmd: Option<String>,
    /// Runner name prefix (names are <prefix>-<i>).
    #[arg(long, default_value = "quod")]
    name: String,
    /// Images to unpack before the first job (repeatable).
    #[arg(long = "image")]
    images: Vec<String>,
    /// observe (default) or replay.
    #[arg(long, default_value = "observe")]
    mode: String,
    /// Identical results a step needs before it replays (the trust layer's
    /// streak, as for `quod run`; 1 under trust v2).
    #[arg(long, default_value_t = quod_trust::DEFAULT_PROOFS)]
    proofs: u32,
    /// Would-be hits of a step key audited in full before any of it replays
    /// blind (trust v2's shadow phase); the rate then decays to --audit.
    #[arg(long, default_value_t = quod_trust::DEFAULT_SHADOW)]
    shadow: u32,
    /// The audit floor: the fraction of would-be hits that execute anyway
    /// and compare (0..=1) once the shadow phase has decayed; 0 turns every
    /// audit off, on purpose only.
    #[arg(long, default_value_t = quod_trust::DEFAULT_AUDIT)]
    audit: f64,
    /// A protected ref besides the default branch whose push and
    /// workflow_dispatch jobs may write the warm build cache and persisted
    /// workspace (`refs/heads/release`; repeatable). Only refs every push to
    /// which is reviewed like the default branch's.
    #[arg(long = "writer-ref")]
    writer_refs: Vec<String>,
    /// Install and register only; do not start.
    #[arg(long)]
    no_start: bool,
}

#[derive(Args)]
pub struct DeregisterArgs {
    #[arg(long)]
    repo: String,
    /// A removal token (repos/{repo}/actions/runners/remove-token).
    #[arg(long, conflicts_with = "token_cmd")]
    token: Option<String>,
    #[arg(long)]
    token_cmd: Option<String>,
}

pub fn runner(state: &State, cmd: RunnerCmd) -> anyhow::Result<u8> {
    match cmd {
        RunnerCmd::Pool(a) => pool(state, a),
        RunnerCmd::Deregister(a) => deregister(state, a),
    }
}

fn runners_dir(state: &State) -> PathBuf {
    state.dir.join("runners")
}

/// A key of versions.toml's `[actions_runner]` section.
fn pinned(key: &str) -> anyhow::Result<String> {
    let mut in_section = false;
    for line in VERSIONS_TOML.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_section = l == "[actions_runner]";
        } else if in_section && let Some(v) = l.strip_prefix(&format!("{key} = ")) {
            return Ok(v.trim_matches('"').to_string());
        }
    }
    bail!("versions.toml has no [actions_runner] {key}")
}

fn pinned_version() -> anyhow::Result<String> {
    pinned("version")
}

/// The file's SHA-256 (sha256sum), hex.
fn sha256(path: &Path) -> anyhow::Result<String> {
    let out = Command::new("sha256sum")
        .arg(path)
        .output()
        .context("sha256sum")?;
    if !out.status.success() {
        bail!("sha256sum {} failed", path.display());
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string())
}

fn arch() -> anyhow::Result<&'static str> {
    Ok(match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => bail!("actions/runner has no build for {other}"),
    })
}

/// The runner release tarball, downloaded once into `<runners>/dist`.
fn dist(base: &Path) -> anyhow::Result<PathBuf> {
    let ver = pinned_version()?;
    let name = format!("actions-runner-linux-{}-{ver}.tar.gz", arch()?);
    let dir = base.join("dist");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(&name);
    let want = pinned(&format!("sha256_{}", arch()?))?;
    if !path.is_file() {
        let url = format!("https://github.com/actions/runner/releases/download/v{ver}/{name}");
        eprintln!("downloading {url}");
        let tmp = path.with_extension("part");
        let st = Command::new("curl")
            .args(["-fsSL", "-o"])
            .arg(&tmp)
            .arg(&url)
            .status()
            .context("curl")?;
        if !st.success() {
            bail!("download of {url} failed");
        }
        std::fs::rename(&tmp, &path)?;
    }
    // Checked every time it is used, not only when downloaded: a tarball
    // replaced on disk is refused too.
    let got = sha256(&path)?;
    if got != want {
        let _ = std::fs::remove_file(&path);
        bail!(
            "{} has SHA-256 {got}, versions.toml pins {want}; removed it",
            path.display()
        );
    }
    Ok(path)
}

fn token(explicit: &Option<String>, cmd: &Option<String>) -> anyhow::Result<String> {
    if let Some(t) = explicit {
        return Ok(t.clone());
    }
    let Some(c) = cmd else {
        bail!("--token or --token-cmd is required to register");
    };
    let out = Command::new("/bin/sh")
        .args(["-c", c])
        .stderr(Stdio::inherit())
        .output()
        .context("token command")?;
    if !out.status.success() {
        bail!("token command failed: {c}");
    }
    let t = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if t.is_empty() {
        bail!("token command printed nothing: {c}");
    }
    Ok(t)
}

fn install(base: &Path, i: u32, a: &PoolArgs, state: &State) -> anyhow::Result<PathBuf> {
    let dir = base.join(format!("runner-{i}"));
    if !dir.join("run.sh").is_file() {
        std::fs::create_dir_all(&dir)?;
        let tarball = dist(base)?;
        let st = Command::new("tar")
            .arg("-xzf")
            .arg(&tarball)
            .arg("-C")
            .arg(&dir)
            .status()
            .context("tar")?;
        if !st.success() {
            bail!("extracting {} failed", tarball.display());
        }
    }
    let hooks = base.join("hooks");
    std::fs::create_dir_all(&hooks)?;
    std::fs::write(hooks.join("index.js"), HOOK_SHIM)?;
    // The shim's configuration: what the hook runs and with which policy.
    // Never the runner's environment, which the step's reaches.
    let exe = std::env::current_exe().context("current exe")?;
    let cfg = serde_json::json!({
        "bin": exe,
        "state": state.dir,
        "policy": {
            "mode": a.mode,
            "proofs": a.proofs,
            "shadow": a.shadow,
            "audit": a.audit,
            "writer_refs": a.writer_refs,
        },
    });
    quod_hooks::Policy::from_json(Some(&cfg["policy"].to_string()))?;
    std::fs::write(hooks.join("quod.json"), serde_json::to_vec_pretty(&cfg)?)?;
    std::fs::write(
        dir.join(".env"),
        format!(
            "ACTIONS_RUNNER_CONTAINER_HOOKS={}\nACTIONS_RUNNER_REQUIRE_JOB_CONTAINER=true\n",
            hooks.join("index.js").display()
        ),
    )?;
    if !dir.join(".runner").is_file() {
        let t = token(&a.token, &a.token_cmd)?;
        let st = Command::new("./config.sh")
            .current_dir(&dir)
            .args([
                "--unattended",
                "--replace",
                "--url",
                &format!("https://github.com/{}", a.repo),
                "--token",
                &t,
                "--name",
                &format!("{}-{i}", a.name),
                "--labels",
                "quod",
                "--work",
                "_work",
            ])
            .env("RUNNER_ALLOW_RUNASROOT", "1")
            .status()
            .context("config.sh")?;
        if !st.success() {
            bail!("config.sh failed for runner {i}");
        }
    }
    Ok(dir)
}

fn pool(state: &State, a: PoolArgs) -> anyhow::Result<u8> {
    if a.mode != "observe" && a.mode != "replay" {
        bail!("--mode {}: observe or replay", a.mode);
    }
    let base = runners_dir(state);
    let t = Instant::now();
    // No job of this state can be running yet: whatever a runner killed
    // with SIGKILL left behind (raw step output, job.json, mounts) goes.
    let swept = quod_hooks::job::sweep_stale_jobs(&state.dir)?;
    if swept > 0 {
        eprintln!("removed {swept} job directory(ies) a killed runner left behind");
    }
    for image in &a.images {
        let r = OciRootfs::new(state.dir.join("rootfs")).prepare(image)?;
        eprintln!("image {image} ready: {} at {}", r.digest, r.dir.display());
    }
    let mut dirs = Vec::new();
    for i in 1..=a.size {
        dirs.push(install(&base, i, &a, state)?);
    }
    eprintln!(
        "{} runner(s) installed and registered in {:.1}s",
        dirs.len(),
        t.elapsed().as_secs_f64()
    );
    if a.no_start {
        return Ok(0);
    }
    // Keep every runner alive; a runner that exits comes back after a pause.
    let mut children: Vec<(PathBuf, Child, Instant)> = Vec::new();
    for d in &dirs {
        children.push((d.clone(), start(d)?, Instant::now()));
    }
    eprintln!("pool of {} warm; ctrl-c stops it", children.len());
    loop {
        std::thread::sleep(Duration::from_secs(2));
        for (dir, child, since) in &mut children {
            if let Some(status) = child.try_wait()? {
                eprintln!(
                    "runner in {} exited ({status}) after {:.0}s; restarting",
                    dir.display(),
                    since.elapsed().as_secs_f64()
                );
                std::thread::sleep(Duration::from_secs(2));
                *child = start(dir)?;
                *since = Instant::now();
            }
        }
    }
}

fn start(dir: &Path) -> anyhow::Result<Child> {
    Command::new("./run.sh")
        .current_dir(dir)
        .env("RUNNER_ALLOW_RUNASROOT", "1")
        .stdin(Stdio::null())
        .spawn()
        .with_context(|| format!("start runner in {}", dir.display()))
}

fn deregister(state: &State, a: DeregisterArgs) -> anyhow::Result<u8> {
    let base = runners_dir(state);
    let mut n = 0;
    if let Ok(rd) = std::fs::read_dir(&base) {
        for e in rd.flatten() {
            let dir = e.path();
            if !dir.join(".runner").is_file() {
                continue;
            }
            let t = token(&a.token, &a.token_cmd)?;
            let st = Command::new("./config.sh")
                .current_dir(&dir)
                .args(["remove", "--token", &t])
                .env("RUNNER_ALLOW_RUNASROOT", "1")
                .status()
                .context("config.sh remove")?;
            if !st.success() {
                bail!("config.sh remove failed in {}", dir.display());
            }
            n += 1;
        }
    }
    eprintln!("{n} runner(s) removed from {}", a.repo);
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_runner_release_is_pinned_by_checksum() {
        assert_eq!(pinned_version().unwrap(), "2.338.2");
        for a in ["x64", "arm64"] {
            let sum = pinned(&format!("sha256_{a}")).unwrap();
            assert!(
                sum.len() == 64 && sum.bytes().all(|b| b.is_ascii_hexdigit()),
                "{a}: {sum}"
            );
        }
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("x");
        std::fs::write(&f, "abc").unwrap();
        assert_eq!(
            sha256(&f).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn version_is_pinned() {
        let v = pinned_version().unwrap();
        assert!(v.starts_with("2."), "{v}");
        assert!(HOOK_SHIM.contains("quod.json"));
        assert!(!HOOK_SHIM.contains("process.env.QUOD"));
    }
}
