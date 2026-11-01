//! Golden step keys: spec in, key out. A changed golden is a schema bump.
//! Regenerate with QUOD_BLESS=1 only alongside a VERSION change
//! (v2: `warm` added to the key, 2026-10-31; v3: `image` became an object
//! with kind, digest, pinned mounts and the platform with its host class,
//! 2026-10-31; v4: `toolchain` added and the env comes from a toolchain
//! profile, 2026-10-31; v5: GOPATH in the Go profile's env, 2026-10-31;
//! v6: the rename to Quod, `.quod/` in the profiles' env, 2026-10-31; v7:
//! the served index and `.git`, what a step is shown, 2026-10-31; v8:
//! served file times in whole microseconds, 2026-11-01).

use quod_cas::Hash;
use quod_key::{Image, ImageKind, Platform, StepSpec, Toolchain, shell_cmd, step_key};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct Golden {
    image: Image,
    toolchain: Toolchain,
    cmd: Vec<String>,
    cwd: String,
    env: Vec<(String, String)>,
    roots: Vec<String>,
    #[serde(default)]
    warm: bool,
    key: Hash,
}

fn cases() -> Vec<StepSpec> {
    // A rootfs pin on a fixed arm64 platform (kernel 6.8, 4 CPUs).
    let img = Image::rootfs(Hash::of(b"quod rootfs go1.27.3 bookworm arm64"));
    let host = Image::new(
        ImageKind::Host,
        Hash::of(b"go1.27.3 linux/arm64 glibc 2.36 debian 12"),
        Platform::new(
            "aarch64",
            "7.0.14-linuxkit",
            "fp asimd; hwcap=0xefffffff hwcap2=0x3fffff",
            18,
            4096,
            vec![("kernel release".into(), "7.0.14-linuxkit".into())],
        ),
    );
    let on = |f: &dyn Fn(&mut Platform)| {
        let mut i = img.clone();
        f(&mut i.platform);
        i
    };
    let x = |pairs: &[(&str, &str)]| {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<Vec<_>>()
    };
    use Toolchain::*;
    let mut out = vec![
        StepSpec::new(Go, img.clone(), "go test ./...", &[]),
        StepSpec::new(Go, img.clone(), "go mod download", &[]),
        StepSpec::new(Go, img.clone(), "go test ./... -run TestFoo", &[]),
        StepSpec::new(
            Go,
            img.clone(),
            "go test ./...",
            &x(&[("GOFLAGS_EXTRA", "-v")]),
        ),
        StepSpec::new(Go, host.clone(), "go test ./...", &[]),
        StepSpec::new(Go, img.clone(), "echo 'quoted \"x\"'\nexit 3", &[]),
        StepSpec::new(
            Go,
            img.clone(),
            "printf '%s' héllo ✓",
            &x(&[("UNICODE", "ü")]),
        ),
        StepSpec::new_warm(Go, img.clone(), "go test ./...", &[]).unwrap(),
        // Same rootfs on another architecture, another kernel series,
        // another CPU count; and with a pinned mount.
        StepSpec::new(
            Go,
            on(&|p| {
                *p = Platform::new(
                    "x86_64",
                    "6.8.0-1021-azure",
                    "fpu sse2 avx2",
                    4,
                    4096,
                    p.host.clone(),
                )
            }),
            "go test ./...",
            &[],
        ),
        StepSpec::new(Go, on(&|p| p.kernel = "6.11".into()), "go test ./...", &[]),
        StepSpec::new(Go, on(&|p| p.cpus = 16), "go test ./...", &[]),
        // Same everything on a host whose kernel build differs.
        StepSpec::new(
            Go,
            on(&|p| p.host[1].1 = "#26-Ubuntu SMP Fri Feb 14 10:01:02 UTC 2025".into()),
            "go test ./...",
            &[],
        ),
        StepSpec::new(
            Go,
            Image {
                mounts: vec![("/opt/tools".into(), Hash::of(b"a tools tree"))],
                ..img.clone()
            },
            "go test ./...",
            &[],
        ),
        StepSpec::new(Node, img.clone(), "npm test", &[]),
        StepSpec::new(Node, img.clone(), "npm ci", &[]),
        StepSpec::new(Python, img.clone(), "python -m pytest", &[]),
        StepSpec::new(
            Python,
            img.clone(),
            "python -m pytest",
            &x(&[("PYTEST_ADDOPTS", "-q")]),
        ),
    ];
    out.push(StepSpec {
        image: img.clone(),
        toolchain: Go,
        cmd: shell_cmd("true"),
        cwd: "/workspace/pkg".into(),
        env: vec![],
        roots: vec!["/workspace".into()],
        warm: false,
    });
    out.push(StepSpec {
        image: img.clone(),
        toolchain: Go,
        cmd: vec!["/usr/bin/env".into(), "go".into(), "vet".into()],
        cwd: "/workspace".into(),
        env: x(&[("B", "2"), ("A", "1")]),
        roots: vec!["/workspace".into(), "/cache".into()],
        warm: false,
    });
    out.push(StepSpec {
        image: img,
        toolchain: Go,
        cmd: vec![],
        cwd: "/".into(),
        env: x(&[("EMPTY", "")]),
        roots: vec![],
        warm: false,
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
            toolchain: s.toolchain,
            cmd: s.cmd,
            cwd: s.cwd,
            env: s.env,
            roots: s.roots,
            warm: s.warm,
        })
        .collect();
    if std::env::var_os("QUOD_BLESS").is_some() {
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
    assert_eq!(committed.len(), 20);
    for g in &committed {
        let spec = StepSpec {
            image: g.image.clone(),
            toolchain: g.toolchain,
            cmd: g.cmd.clone(),
            cwd: g.cwd.clone(),
            env: g.env.clone(),
            roots: g.roots.clone(),
            warm: g.warm,
        };
        assert_eq!(
            step_key(&spec).unwrap(),
            g.key,
            "golden changed for {spec:?}"
        );
    }
    assert_eq!(committed, actual);
}
