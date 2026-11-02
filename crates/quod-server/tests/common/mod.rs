//! A quod-server in-process and "runners": each a state dir of its own (a
//! local CAS) speaking quod.v1 through `RemoteLedger`, the client `quod
//! run` and the hooks use. A step here is synthetic (no sandbox): its
//! inputs are a tree, it reads every file of it, and its result is the
//! stdout it is given, so these suites run anywhere the server builds.
#![allow(dead_code)]

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use quod_cas::{Cas, Entry, Hash, Kind, Tree};
use quod_ledger::{RunRecord, TrustState};
use quod_manifest::{Access, Manifest, StepResult};
use quod_server::config::{Document, Settings, parse_document};
use quod_trust::{Claimed, Mode, Policy, Reason, Step, StepLedger, Verdict};
use quod_wire::client::{Auth, RemoteLedger, ServerConfig};

pub const ACME_RUNNER: &str = "acme-runner-token";
pub const ACME_FORK: &str = "acme-fork-token";
pub const ACME_DELEGATE: &str = "acme-delegate-token";
pub const ACME_VIEWER: &str = "acme-viewer-token";
pub const BETA_RUNNER: &str = "beta-runner-token";
pub const BETA_VIEWER: &str = "beta-viewer-token";

pub const ACME_OWNER: u64 = 42;
pub const BETA_OWNER: u64 = 43;

pub fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn h(token: &str) -> String {
    blake3::hash(token.as_bytes()).to_hex().to_string()
}

/// Teams whose policy floor is `team_proofs` and no audits (so the
/// runner's own policy decides audits and the shadow phase).
pub fn document(team_proofs: u32) -> Document {
    document_with(serde_json::json!({"proofs": team_proofs, "shadow": 0, "audit": 0.0}))
}

pub fn document_with(team_policy: serde_json::Value) -> Document {
    let j = serde_json::json!({
        "teams": [
            {"slug": "acme", "name": "Acme", "github_owner_ids": [ACME_OWNER],
             "policy": team_policy},
            {"slug": "beta", "github_owner_ids": [BETA_OWNER],
             "policy": team_policy},
        ],
        "tokens": [
            {"blake3": h(ACME_RUNNER), "team": "acme", "kind": "runner", "repository": "acme/app"},
            {"blake3": h(ACME_FORK), "team": "acme", "kind": "runner", "repository": "acme/app", "trust": "fork-evil_app"},
            {"blake3": h(ACME_DELEGATE), "team": "acme", "kind": "runner", "delegate": true},
            {"blake3": h(ACME_VIEWER), "team": "acme", "kind": "viewer", "role": "member"},
            {"blake3": h(BETA_RUNNER), "team": "beta", "kind": "runner", "repository": "acme/app"},
            {"blake3": h(BETA_VIEWER), "team": "beta", "kind": "viewer"},
        ],
        "oidc": {"audiences": ["quodci"], "jwks_file": fixture("test-oidc-jwks.json")},
    });
    parse_document(&j.to_string()).unwrap()
}

pub const SIGNING_SEED: &str = "5151515151515151515151515151515151515151515151515151515151515151";

pub struct Server {
    pub url: String,
    pub data: tempfile::TempDir,
    pub rt: tokio::runtime::Runtime,
}

impl Server {
    pub fn start(team_proofs: u32) -> Server {
        Server::start_with(document(team_proofs))
    }

    /// A server whose teams take trust v2's default policy (proofs 1, the
    /// shadow phase, the 5% floor).
    pub fn start_v2() -> Server {
        Server::start_with(document_with(serde_json::json!({})))
    }

    pub fn start_with(doc: Document) -> Server {
        let data = tempfile::tempdir().unwrap();
        let settings = Settings {
            bind: "127.0.0.1".into(),
            port: 0,
            data_dir: data.path().join("data"),
            signing_key: quod_wire::signing_key_from_hex(SIGNING_SEED).unwrap(),
            doc,
            sse_hold: Duration::from_secs(3),
            max_blob: 64 << 20,
            max_json: 64 << 20,
        };
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .unwrap();
        let (url, _) = rt.block_on(quod_server::spawn_local(settings)).unwrap();
        Server { url, data, rt }
    }

    pub fn public_key(&self) -> String {
        hex::encode(
            quod_wire::signing_key_from_hex(SIGNING_SEED)
                .unwrap()
                .verifying_key()
                .to_bytes(),
        )
    }

    /// A blocking GET with a bearer token: (status, body).
    pub fn get(&self, path: &str, token: &str) -> (u16, String) {
        let r = ureq::get(&format!("{}{path}", self.url))
            .set("quod-protocol", "quod.v1")
            .set("authorization", &format!("Bearer {token}"))
            .call();
        match r {
            Ok(r) => (r.status(), r.into_string().unwrap()),
            Err(ureq::Error::Status(c, r)) => (c, r.into_string().unwrap_or_default()),
            Err(e) => panic!("{e}"),
        }
    }
}

/// One runner: its own state dir and local CAS, and how it authenticates.
pub struct Runner {
    pub dir: tempfile::TempDir,
    pub cas: Cas,
    pub cfg: ServerConfig,
    pub policy: Policy,
}

/// A replay-mode policy without trust v2's shadow phase, so a test's
/// replays and audits follow the floor alone (`policy_v2` has the shadow).
pub fn policy(proofs: u32, audit: f64) -> Policy {
    Policy {
        mode: Mode::Replay,
        proofs,
        shadow: 0,
        audit,
        replay_failures: false,
    }
}

impl Runner {
    pub fn new(server: &Server, auth: Auth) -> Runner {
        Runner::at(&server.url, Some(&server.public_key()), auth)
    }

    pub fn at(url: &str, key: Option<&str>, auth: Auth) -> Runner {
        let dir = tempfile::tempdir().unwrap();
        let cas = Cas::open(dir.path().join("cas")).unwrap();
        Runner {
            cas,
            dir,
            cfg: ServerConfig {
                url: url.to_string(),
                auth,
                server_key: key.map(|k| quod_wire::verifying_key_from_hex(k).unwrap()),
                timeout: Duration::from_secs(20),
                scope: None,
                ci: Some(quod_wire::CiContext {
                    repository: Some("acme/app".into()),
                    run_number: Some(1),
                    job: Some("test".into()),
                    step_name: Some("go test".into()),
                    ..Default::default()
                }),
            },
            policy: policy(2, 0.0),
        }
    }

    pub fn token(server: &Server, token: &str) -> Runner {
        Runner::new(server, Auth::Token(token.into()))
    }

    pub fn ledger(&self) -> RemoteLedger<'_> {
        RemoteLedger::new(self.cfg.clone(), &self.cas, self.policy)
    }
}

/// A workspace tree of `files` (path, content).
pub fn tree(files: &[(&str, &str)]) -> Tree {
    Tree::from_leaves(
        files
            .iter()
            .map(|(p, c)| Entry::file(*p, Hash::of(c.as_bytes()), 0o644)),
    )
}

pub fn key_of(cmd: &str) -> Hash {
    Hash::of(format!("synthetic step key: {cmd}").as_bytes())
}

/// What a step reads: every file and directory of the tree, and one path
/// it looked for and did not find.
pub fn manifest(key: Hash, t: &Tree) -> Manifest {
    let mut log: Vec<Access> = t
        .iter()
        .map(|e| Access {
            path: e.p.clone(),
            kind: e.k,
        })
        .collect();
    log.push(Access {
        path: "config.local".into(),
        kind: Kind::Absent,
    });
    Manifest::from_log(key, vec!["/workspace".into()], t, log).unwrap()
}

pub fn run_id() -> String {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{t:026}{n:06}")
}

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum Outcome {
    /// Replayed: the stdout it served (read from the runner's own CAS).
    Replayed(Vec<u8>),
    Executed(Verdict),
    /// The claim said execute; nothing was committed (`claim_only`).
    WouldExecute(Reason),
}

impl Outcome {
    pub fn replayed(&self) -> bool {
        matches!(self, Outcome::Replayed(_))
    }
}

/// A claim only: does this tree replay?
pub fn claim(r: &Runner, cmd: &str, t: &Tree) -> Outcome {
    let l = r.ledger();
    let key = key_of(cmd);
    let step = Step {
        key,
        cmd,
        ignore: &[],
    };
    let rid = run_id();
    let (lookup, claimed) = l
        .claim(&step, t, &mut |_| RunRecord {
            id: rid.clone(),
            step_key: Some(key),
            wall_ms: Some(5),
            ..RunRecord::default()
        })
        .unwrap();
    match claimed {
        Claimed::Replay { .. } => {
            let (_, res) = lookup.hit.unwrap();
            Outcome::Replayed(std::fs::read(r.cas.blob_path(&res.stdout)).unwrap())
        }
        Claimed::Execute { reason } => Outcome::WouldExecute(reason),
    }
}

/// Trust v2's default policy in replay mode: proofs 1, a shadow of 2,
/// the 5% floor.
pub fn policy_v2() -> Policy {
    Policy {
        mode: Mode::Replay,
        ..Policy::default()
    }
}

/// One step: claim, and on anything but a replay "execute" (produce
/// `stdout`) and commit.
pub fn step(r: &Runner, cmd: &str, t: &Tree, stdout: &str) -> Outcome {
    let l = r.ledger();
    let o = drive(&l, &r.cas, cmd, t, stdout);
    assert!(l.is_online(), "the server went offline during a commit");
    o
}

/// `step` on any `StepLedger`: the server's (`RemoteLedger`) or a local
/// one (`LocalLedger`), so one scenario can run on both.
pub fn drive(l: &dyn StepLedger, cas: &Cas, cmd: &str, t: &Tree, stdout: &str) -> Outcome {
    let key = key_of(cmd);
    let step = Step {
        key,
        cmd,
        ignore: &[],
    };
    let rid = run_id();
    let (lookup, claimed) = l
        .claim(&step, t, &mut |hit| RunRecord {
            id: rid.clone(),
            step_key: Some(key),
            outcome: Some("replayed".into()),
            manifest_id: Some(hit.manifest_id),
            exit_code: Some(hit.exit_code),
            wall_ms: Some(5),
            ..RunRecord::default()
        })
        .unwrap();
    let reason = match claimed {
        Claimed::Replay { .. } => {
            let (_, res) = lookup.hit.unwrap();
            return Outcome::Replayed(std::fs::read(cas.blob_path(&res.stdout)).unwrap());
        }
        Claimed::Execute { reason } => reason,
    };
    let m = manifest(key, t);
    let mid = quod_manifest::manifest_id(&m).unwrap();
    let out = cas.put(stdout.as_bytes()).unwrap();
    let err = cas.put(b"").unwrap();
    let res = StepResult {
        v: quod_manifest::VERSION,
        step_key: key,
        manifest_id: mid,
        run_id: rid.clone(),
        recorded_at: quod_ledger::now_rfc3339(),
        exit_code: 0,
        stdout: out,
        stderr: err,
        outputs: vec![],
        root_outputs: vec![],
        duration_ms: 17_000,
        commands: vec![],
    };
    let v = l
        .commit_execution(
            &step,
            &rid,
            &m,
            &res,
            lookup.hit.as_ref(),
            &reason,
            RunRecord {
                id: rid.clone(),
                step_key: Some(key),
                manifest_id: Some(mid),
                exit_code: Some(0),
                wall_ms: Some(17_000),
                mode: Some("replay".into()),
                ..RunRecord::default()
            },
        )
        .unwrap();
    Outcome::Executed(v)
}

pub fn proven(o: &Outcome) -> bool {
    matches!(o, Outcome::Executed(v) if v.trust.state == TrustState::Proven)
}
