//! Golden step keys: spec in, key out. A changed golden is a schema bump.
//! Regenerate with STILLPASS_BLESS=1 only alongside a VERSION change.

use serde::{Deserialize, Serialize};
use sp_cas::Hash;
use sp_key::{StepSpec, shell_cmd, step_key};

#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct Golden {
    image: Hash,
    cmd: Vec<String>,
    cwd: String,
    env: Vec<(String, String)>,
    roots: Vec<String>,
    key: Hash,
}

fn cases() -> Vec<StepSpec> {
    let img = Hash::of(b"go1.27.3 linux/arm64 glibc 2.36 debian 12");
    let x = |pairs: &[(&str, &str)]| {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<Vec<_>>()
    };
    let mut out = vec![
        StepSpec::spike(img, "go test ./...", &[]),
        StepSpec::spike(img, "go mod download", &[]),
        StepSpec::spike(img, "go test ./... -run TestFoo", &[]),
        StepSpec::spike(img, "go test ./...", &x(&[("GOFLAGS_EXTRA", "-v")])),
        StepSpec::spike(Hash::of(b"rootfs"), "go test ./...", &[]),
        StepSpec::spike(img, "echo 'quoted \"x\"'\nexit 3", &[]),
        StepSpec::spike(img, "printf '%s' héllo ✓", &x(&[("UNICODE", "ü")])),
    ];
    out.push(StepSpec {
        image: img,
        cmd: shell_cmd("true"),
        cwd: "/workspace/pkg".into(),
        env: vec![],
        roots: vec!["/workspace".into()],
    });
    out.push(StepSpec {
        image: img,
        cmd: vec!["/usr/bin/env".into(), "go".into(), "vet".into()],
        cwd: "/workspace".into(),
        env: x(&[("B", "2"), ("A", "1")]),
        roots: vec!["/workspace".into(), "/cache".into()],
    });
    out.push(StepSpec {
        image: img,
        cmd: vec![],
        cwd: "/".into(),
        env: x(&[("EMPTY", "")]),
        roots: vec![],
    });
    out
}

#[test]
fn goldens_match() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/step_keys.jsonl"
    );
    let actual: Vec<Golden> = cases()
        .into_iter()
        .map(|s| Golden {
            key: step_key(&s).unwrap(),
            image: s.image,
            cmd: s.cmd,
            cwd: s.cwd,
            env: s.env,
            roots: s.roots,
        })
        .collect();
    if std::env::var_os("STILLPASS_BLESS").is_some() {
        let text: String = actual
            .iter()
            .map(|g| serde_json::to_string(g).unwrap() + "\n")
            .collect();
        std::fs::write(path, text).unwrap();
    }
    let committed: Vec<Golden> = std::fs::read_to_string(path)
        .expect("fixtures/step_keys.jsonl missing")
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(committed.len(), 10);
    for g in &committed {
        let spec = StepSpec {
            image: g.image,
            cmd: g.cmd.clone(),
            cwd: g.cwd.clone(),
            env: g.env.clone(),
            roots: g.roots.clone(),
        };
        assert_eq!(
            step_key(&spec).unwrap(),
            g.key,
            "golden changed for {spec:?}"
        );
    }
    assert_eq!(committed, actual);
}
