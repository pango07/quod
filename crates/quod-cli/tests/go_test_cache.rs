//! Go's test-result cache in a warm step, under Quod's guard
//! (quod_key::gotest), end to end through the binary. Gated on
//! QUOD_INTEGRATION=1 like the rest.
//!
//! G1 an audit (and a re-proof) runs every test: go test serves nothing
//! from its cache there. G2 go test keeps its cache for a test whose every
//! input it rechecks, and refuses it for the ones it would not: a process
//! the test started, scratch it did not write, a file outside the module.
//! G3 the inputs plain go test's cache misses (a TestMain's read before
//! m.Run, a test file's package variable, a file outside the module, a
//! subprocess's read, a scratch file the script wrote) each change a
//! result, and a changed one is never served stale. G4 another toolchain in
//! the workspace is a tainted run, never recorded.

mod common;
use common::*;

/// The packages, each with a test that passes while its input is `v1`.
const PKGS: &[(&str, &str)] = &[
    // Read by the test itself: go test rechecks it (the control).
    (
        "a/a_test.go",
        r#"package a
import ("os"; "strings"; "testing")
func TestA(t *testing.T) {
	b, _ := os.ReadFile("testdata/x.txt")
	if strings.TrimSpace(string(b)) != "v1" { t.Fatalf("x: %q", b) }
}
"#,
    ),
    // Read by TestMain before m.Run starts go's own log.
    (
        "e/e_test.go",
        r#"package e
import ("os"; "strings"; "testing")
var y string
func TestMain(m *testing.M) { b, _ := os.ReadFile("testdata/y.txt"); y = strings.TrimSpace(string(b)); os.Exit(m.Run()) }
func TestE(t *testing.T) { if y != "v1" { t.Fatalf("y: %q", y) } }
"#,
    ),
    // Read by a test file's package variable, at init.
    (
        "f/f_test.go",
        r#"package f
import ("os"; "strings"; "testing")
func read(p string) string { b, _ := os.ReadFile(p); return strings.TrimSpace(string(b)) }
var z = read("testdata/z.txt")
func TestF(t *testing.T) { if z != "v1" { t.Fatalf("z: %q", z) } }
"#,
    ),
    // In the workspace but outside the module, which go test never rechecks.
    (
        "g/g_test.go",
        r#"package g
import ("os"; "strings"; "testing")
func TestG(t *testing.T) {
	b, _ := os.ReadFile("../../shared.txt")
	if strings.TrimSpace(string(b)) != "v1" { t.Fatalf("shared: %q", b) }
}
"#,
    ),
    // Read by a process the test starts: in no test log.
    (
        "b/b_test.go",
        r#"package b
import ("os/exec"; "strings"; "testing")
func TestB(t *testing.T) {
	out, err := exec.Command("/bin/cat", "../../sub.txt").Output()
	if err != nil || strings.TrimSpace(string(out)) != "v1" { t.Fatalf("sub: %q %v", out, err) }
}
"#,
    ),
    // Scratch the step's script wrote before go test ran.
    (
        "c/c_test.go",
        r#"package c
import ("os"; "strings"; "testing")
func TestC(t *testing.T) {
	b, _ := os.ReadFile("/tmp/quod-g3.txt")
	if strings.TrimSpace(string(b)) != "v1" { t.Fatalf("tmp: %q", b) }
}
"#,
    ),
    // Its own t.TempDir: created by the test, cacheable.
    (
        "d/d_test.go",
        r#"package d
import ("os"; "path/filepath"; "testing")
func TestD(t *testing.T) {
	p := filepath.Join(t.TempDir(), "f")
	os.WriteFile(p, []byte("d"), 0o644)
	if b, _ := os.ReadFile(p); string(b) != "d" { t.Fatal("d") }
}
"#,
    ),
];

/// Each package's input, relative to the repo.
const INPUTS: &[(&str, &str)] = &[
    ("a", "mod/a/testdata/x.txt"),
    ("e", "mod/e/testdata/y.txt"),
    ("f", "mod/f/testdata/z.txt"),
    ("g", "shared.txt"),
    ("b", "sub.txt"),
    ("c", "data.txt"),
];

const STEP: &str = "cp data.txt /tmp/quod-g3.txt && cd mod && go test ./...";

fn module() -> Env {
    let env = Env::new();
    // Replace the tiny module with one nested under mod/, so a file in the
    // workspace can be outside the module.
    for p in ["go.mod", "calc", "other"] {
        let _ = std::fs::remove_dir_all(env.repo.join(p));
        let _ = std::fs::remove_file(env.repo.join(p));
    }
    env.write("mod/go.mod", "module example.com/g\n\ngo 1.27\n");
    for (p, src) in PKGS {
        env.write(&format!("mod/{p}"), src);
    }
    for (_, input) in INPUTS {
        env.write(input, "v1\n");
    }
    env.commit("g: the guard's module");
    env
}

/// Per package: `ok` served from the cache, `ok` run, or `FAIL`.
fn outcomes(stdout: &[u8]) -> Vec<(String, String)> {
    String::from_utf8_lossy(stdout)
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            let pkg = f.get(1)?.strip_prefix("example.com/g/")?.to_string();
            let what = match (f[0].trim(), f.get(2)) {
                ("ok", Some(&"(cached)")) => "cached",
                ("ok", _) => "ran",
                ("FAIL", _) => "FAIL",
                _ => return None,
            };
            Some((pkg, what.to_string()))
        })
        .collect()
}

fn of<'a>(o: &'a [(String, String)], pkg: &str) -> &'a str {
    o.iter()
        .find(|(p, _)| p == pkg)
        .map_or("missing", |(_, w)| w.as_str())
}

#[test]
fn g1_to_g4_go_test_caches_only_what_it_rechecks() {
    if !enabled() {
        return;
    }
    let mut env = module();
    env.run_defaults = ["--warm"].map(String::from).to_vec();

    // G1: replay mode, every would-be hit audited. The first run executes;
    // the second matches it and is the audit, with go test's cache expired.
    let audit = ["--mode", "replay", "--proofs", "1", "--audit", "1"];
    let r1 = env.run_step(STEP, &audit);
    assert!(r1.executed() && r1.code == 0, "G1 first run: {}", r1.stderr);
    assert!(
        r1.stderr.contains("warm build cache"),
        "G1: a warm step: {}",
        r1.stderr
    );
    let r2 = env.run_step(STEP, &audit);
    let o2 = outcomes(&r2.stdout);
    assert!(r2.executed() && r2.code == 0, "G1 audit: {}", r2.stderr);
    assert!(
        o2.len() == PKGS.len() && o2.iter().all(|(_, w)| w == "ran"),
        "G1: an audit runs every test: {o2:?}"
    );
    assert!(!r2.stderr.contains("DIVERGENCE"), "G1: {}", r2.stderr);
    println!(
        "G1 ok: the audit ran all {} packages, none from go test's cache",
        o2.len()
    );

    // G2: observe mode executes every run, with go test's cache.
    let r3 = env.run_step(STEP, &[]);
    let o3 = outcomes(&r3.stdout);
    assert_eq!(r3.code, 0, "G2: {}", r3.stderr);
    for (pkg, want) in [
        ("a", "cached"),
        ("e", "cached"),
        ("f", "cached"),
        ("d", "cached"),
        ("g", "ran"),
        ("b", "ran"),
        ("c", "ran"),
    ] {
        assert_eq!(of(&o3, pkg), want, "G2 {pkg}: {o3:?}");
    }
    println!("G2 ok: {o3:?}");

    // G3: each input changed; its package must fail, never be served.
    for (_, input) in INPUTS {
        env.write(input, "v2\n");
    }
    env.commit("g: every input changed");
    let r4 = env.run_step(STEP, &[]);
    let o4 = outcomes(&r4.stdout);
    for (pkg, _) in INPUTS {
        assert_eq!(of(&o4, pkg), "FAIL", "G3 {pkg} must fail: {o4:?}");
    }
    assert_eq!(of(&o4, "d"), "cached", "G3 d: {o4:?}");
    // And back: they pass again (from the cache where go test rechecks).
    for (_, input) in INPUTS {
        env.write(input, "v1\n");
    }
    env.commit("g: back to v1");
    let r5 = env.run_step(STEP, &[]);
    assert_eq!(r5.code, 0, "G3 back: {:?}", outcomes(&r5.stdout));
    println!("G3 ok: every changed input failed its package: {o4:?}");

    // G4: a second Go toolchain inside the workspace is not guarded.
    env.write(
        ".quod-sdk/go/src/testing/internal/testdeps/deps.go",
        "package testdeps\n",
    );
    env.commit("g: an SDK in the workspace");
    let r6 = env.run_step(
        &format!("cat .quod-sdk/go/src/testing/internal/testdeps/deps.go >/dev/null && {STEP}"),
        &[],
    );
    assert!(
        r6.stderr
            .contains("ran outside the test-cache guard, not memoized"),
        "G4: {}",
        r6.stderr
    );
    println!("G4 ok: a workspace toolchain's run is tainted");
}
