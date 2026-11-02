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
    document_json(team_policy, None)
}

/// The WorkOS client id the test documents configure, and the organizations
/// of the two teams.
pub const WORKOS_CLIENT: &str = "client_test";
pub const ACME_ORG: &str = "org_acme";
pub const BETA_ORG: &str = "org_beta";

/// The test document; `workos_api` points the membership fallback at a
/// local stand-in for WorkOS's API.
pub fn document_json(team_policy: serde_json::Value, workos_api: Option<&str>) -> Document {
    let j = serde_json::json!({
        "teams": [
            {"slug": "acme", "name": "Acme", "github_owner_ids": [ACME_OWNER],
             "workos_org_id": ACME_ORG,
             "repos": [{"full_name": "acme/app", "default_branch": "trunk"}],
             "policy": team_policy},
            {"slug": "beta", "github_owner_ids": [BETA_OWNER], "workos_org_id": BETA_ORG,
             "policy": team_policy},
        ],
        "viewers": {"workos": {"client_id": WORKOS_CLIENT,
                               "jwks_file": fixture("test-oidc-jwks.json"),
                               "api_url": workos_api}},
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

/// Whether the suites run on Postgres and S3: `QUOD_TEST_DATABASE_URL`
/// (a Postgres the tests may create schemas in) and `QUOD_TEST_S3_ENDPOINT`,
/// `QUOD_TEST_S3_BUCKET`, `QUOD_TEST_S3_ACCESS_KEY_ID`,
/// `QUOD_TEST_S3_SECRET_ACCESS_KEY` (a MinIO in CI, path-style). Without
/// them every server here is SQLite on a temporary data dir.
pub fn cloud() -> bool {
    std::env::var("QUOD_TEST_DATABASE_URL").is_ok_and(|v| !v.is_empty())
}

fn need(k: &str) -> String {
    std::env::var(k).unwrap_or_else(|_| panic!("{k} is needed with QUOD_TEST_DATABASE_URL"))
}

/// A name no other test uses.
pub fn unique(prefix: &str) -> String {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{prefix}_{}_{t}_{n}", std::process::id())
}

/// Fresh cloud settings: a Postgres schema of its own (created here) and
/// an object prefix of its own, so tests never see each other's rows or
/// blobs while sharing one database and bucket.
pub fn cloud_env() -> quod_server::config::CloudEnv {
    let url = need("QUOD_TEST_DATABASE_URL");
    let schema = unique("t");
    let mut c = r2d2_postgres::postgres::Client::connect(&url, r2d2_postgres::postgres::NoTls)
        .expect("the test Postgres");
    c.batch_execute(&format!("CREATE SCHEMA {schema}")).unwrap();
    let s3 = quod_server::s3::S3Settings {
        endpoint: need("QUOD_TEST_S3_ENDPOINT"),
        bucket: need("QUOD_TEST_S3_BUCKET"),
        region: "us-east-1".into(),
        access_key_id: need("QUOD_TEST_S3_ACCESS_KEY_ID"),
        secret_access_key: need("QUOD_TEST_S3_SECRET_ACCESS_KEY"),
        path_style: true,
        prefix: format!("{schema}/"),
        part_size: quod_server::s3::MIN_PART_SIZE,
    };
    quod_server::s3::S3::new(s3.clone())
        .unwrap()
        .create_bucket()
        .unwrap();
    quod_server::config::CloudEnv {
        pg: quod_server::pg::PgSettings {
            url,
            pool_size: 16,
            options: Some(format!("-c search_path={schema}")),
        },
        s3,
    }
}

/// Connected cloud backends on fresh settings.
pub fn fresh_cloud() -> std::sync::Arc<quod_server::store::Cloud> {
    std::sync::Arc::new(quod_server::boot::connect(&cloud_env()).unwrap())
}

pub struct Server {
    pub url: String,
    pub data: tempfile::TempDir,
    pub rt: tokio::runtime::Runtime,
    /// The server's store (its backend), for looking at what it wrote.
    pub store: quod_server::store::Store,
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
        Server::start_full(doc, None)
    }

    pub fn start_full(doc: Document, workos_api_key: Option<&str>) -> Server {
        Server::start_backend(
            doc,
            workos_api_key,
            cloud().then(fresh_cloud),
            tempfile::tempdir().unwrap(),
        )
    }

    /// A server on the given backend (`None`: SQLite under `data`).
    pub fn start_backend(
        doc: Document,
        workos_api_key: Option<&str>,
        backend: Option<std::sync::Arc<quod_server::store::Cloud>>,
        data: tempfile::TempDir,
    ) -> Server {
        let settings = Settings {
            bind: "127.0.0.1".into(),
            port: 0,
            data_dir: data.path().join("data"),
            signing_key: quod_wire::signing_key_from_hex(SIGNING_SEED).unwrap(),
            doc,
            sse_hold: Duration::from_secs(3),
            max_blob: 128 << 20,
            max_json: 64 << 20,
            workos_api_key: workos_api_key.map(String::from),
            cloud: backend,
        };
        Server::on(settings, data)
    }

    /// A server over `settings` (its data dir and signing key), keeping
    /// `data` alive as long as it runs.
    pub fn on(settings: Settings, data: tempfile::TempDir) -> Server {
        // The server's log in a failing test's output (an internal error's
        // cause is only logged).
        let _ = tracing_subscriber::fmt()
            .with_test_writer()
            .with_env_filter("quod=warn")
            .try_init();
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .unwrap();
        let store = match &settings.cloud {
            Some(c) => quod_server::store::Store::cloud(&settings.data_dir, c.clone()),
            None => quod_server::store::Store::new(&settings.data_dir),
        };
        let (url, _) = rt.block_on(quod_server::spawn_local(settings)).unwrap();
        Server {
            url,
            data,
            rt,
            store,
        }
    }

    /// Stops the server (its tasks and connections end with the runtime)
    /// and hands back its data dir.
    pub fn stop(self) -> tempfile::TempDir {
        let Server { data, rt, .. } = self;
        // (`store` goes with it; a cloud store's pool closes when the last
        // clone does.)
        rt.shutdown_timeout(Duration::from_secs(5));
        data
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
        defer: false,
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
        .claim(
            &step,
            t,
            &mut |_| RunRecord {
                id: rid.clone(),
                step_key: Some(key),
                wall_ms: Some(5),
                ..RunRecord::default()
            },
            None,
        )
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
        .claim(
            &step,
            t,
            &mut |hit| RunRecord {
                id: rid.clone(),
                step_key: Some(key),
                outcome: Some("replayed".into()),
                manifest_id: Some(hit.manifest_id),
                exit_code: Some(hit.exit_code),
                wall_ms: Some(5),
                ..RunRecord::default()
            },
            None,
        )
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
