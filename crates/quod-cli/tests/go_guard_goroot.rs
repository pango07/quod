//! The go test cache guard on a toolchain whose GOROOT is reached through a
//! symlink (MAJOR 1 of the review of 65fa744): the guard's overlay must name
//! GOROOT exactly as the step's go command spells it, which cmd/go decides
//! (`go env GOROOT`), not a path Quod derives from PATH.
//!
//! The profile's PATH is fixed at /usr/local/go/bin, so each command here
//! runs in a private mount namespace (`unshare -m`) where /usr/local is a
//! directory of the test's holding `go -> /usr/local/realgo`, the host's
//! toolchain bound there: under /usr, which the sandbox binds, as an
//! install's symlink would be. The same directory every time, so the host
//! fingerprint (which lists /usr/local/go) is the same for every run.
//! Nothing outside the namespace changes. Gated on
//! QUOD_INTEGRATION=1; skipped without unshare or the toolchain.

mod common;
use common::*;

use std::path::Path;
use std::process::Command;

/// Run in the namespace with $1 the directory to show at /usr/local (it
/// holds `go -> /usr/local/realgo` and an empty `realgo`): the toolchain is
/// bound onto its `realgo`, then it onto /usr/local. The rest is the
/// command.
const LINKED: &str = r#"set -e
d=$1
shift
mount --bind /usr/local/go "$d/realgo"
mount --rbind "$d" /usr/local
exec "$@""#;

fn linked(dir: &Path, args: &[&str]) -> Command {
    let mut c = Command::new("unshare");
    c.args([
        "-m",
        "--propagation",
        "private",
        "/bin/sh",
        "-c",
        LINKED,
        "sh",
    ])
    .arg(dir)
    .args(args);
    c
}

/// The directory shown at /usr/local, made once.
fn usr_local(env: &Env) -> std::path::PathBuf {
    let d = env.tmp.path().join("usr-local");
    if !d.exists() {
        std::fs::create_dir_all(d.join("realgo")).unwrap();
        std::os::unix::fs::symlink("/usr/local/realgo", d.join("go")).unwrap();
    }
    d
}

/// `quod run` in the namespace.
fn run(env: &Env, step: &str, extra: &[&str]) -> Out {
    seed_rootfs_in(&env.state());
    let out = linked(&usr_local(env), &[env!("CARGO_BIN_EXE_quod")])
        .arg("--state")
        .arg(env.state())
        .args(["run", "--repo", env.repo.to_str().unwrap(), "--step", step])
        .args(["--toolchain", "go", "--warm"])
        .args(extra)
        .output()
        .unwrap();
    let o = Out {
        code: out.status.code().unwrap_or(-1),
        stdout: out.stdout,
        stderr: String::from_utf8_lossy(&out.stderr).into(),
    };
    eprintln!("$ quod run --step {step} {}\n{}", extra.join(" "), o.stderr);
    o
}

/// Per package: `cached`, `ran` or what go test printed first.
fn outcome(stdout: &[u8], pkg: &str) -> String {
    String::from_utf8_lossy(stdout)
        .lines()
        .find_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            (f.get(1) == Some(&pkg)).then(|| match (f[0].trim(), f.get(2)) {
                ("ok", Some(&"(cached)")) => "cached".to_string(),
                ("ok", _) => "ran".to_string(),
                (other, _) => other.to_string(),
            })
        })
        .unwrap_or_else(|| "missing".into())
}

#[test]
fn a_symlinked_goroot_gets_the_guard_at_the_goroot_go_reports() {
    if !enabled() || !Path::new("/usr/local/go/bin/go").is_file() {
        return;
    }
    let env = Env::new();
    let probe = linked(&usr_local(&env), &["/usr/local/go/bin/go", "env", "GOROOT"])
        .env("GOTOOLCHAIN", "local")
        .output();
    let reported = match probe {
        Ok(o) if o.status.success() => String::from_utf8(o.stdout).unwrap().trim().to_string(),
        other => {
            eprintln!("no private mount namespace here ({other:?}); skipping");
            return;
        }
    };
    assert_eq!(
        reported, "/usr/local/realgo",
        "cmd/go spells GOROOT by the directory its binary is in"
    );

    for p in ["go.mod", "calc", "other"] {
        let _ = std::fs::remove_dir_all(env.repo.join(p));
        let _ = std::fs::remove_file(env.repo.join(p));
    }
    env.write("go.mod", "module example.com/g\n\ngo 1.27\n");
    env.write(
        "a/a_test.go",
        r#"package a
import ("os"; "testing")
func TestA(t *testing.T) { if _, err := os.ReadFile("testdata/x.txt"); err != nil { t.Fatal(err) } }
"#,
    );
    env.write("a/testdata/x.txt", "v1\n");
    env.commit("g: a small module");
    let step = "go test ./...";

    let r1 = run(&env, step, &["--keep"]);
    assert_eq!(r1.code, 0, "{}", r1.stderr);
    let overlay = std::fs::read_dir(env.state().join("runs"))
        .unwrap()
        .flatten()
        .map(|e| e.path().join("go-test-guard/overlay.json"))
        .find(|p| p.is_file())
        .expect("the guard applies: the kept run has its files");
    let overlay: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(overlay).unwrap()).unwrap();
    let keys: Vec<&String> = overlay["Replace"].as_object().unwrap().keys().collect();
    assert_eq!(keys.len(), 2, "one spelling of GOROOT: {keys:?}");
    assert!(
        keys.iter().all(|k| k.starts_with(&format!("{reported}/"))),
        "the overlay names GOROOT as go reports it ({reported}): {keys:?}"
    );
    // And it applies in the step: after an unrelated change (a miss), go
    // test serves package a from its cache.
    env.write("README", "x\n");
    env.commit("g: unrelated");
    let r2 = run(&env, step, &[]);
    assert_eq!(r2.code, 0, "{}", r2.stderr);
    assert_eq!(outcome(&r2.stdout, "example.com/g/a"), "cached");
    println!("ok: through a symlinked /usr/local/go, the overlay keyed {reported}");
}
