//! Go's test-result cache in a warm step, under Quod's guard
//! (quod_key::gotest), end to end through the binary. Gated on
//! QUOD_INTEGRATION=1 like the rest.
//!
//! G1 an audit (and a re-proof) runs every test: go test serves nothing
//! from its cache there. G2 go test keeps its cache, in a run that only
//! records (a miss), for a test whose every
//! input it rechecks, and refuses it for the ones it would not: a process
//! the test started, scratch it did not write, a file outside the module.
//! G3 the inputs plain go test's cache misses (a TestMain's read before
//! m.Run, a test file's package variable, a file outside the module, a
//! subprocess's read, a scratch file the script wrote) each change a
//! result, and a changed one is never served stale. G4 another toolchain in
//! the workspace is a tainted run, never recorded. G5 every execution trust
//! v2 takes as evidence (a proof, a shadow audit, an observe-mode would-be
//! hit) runs every test, and only the miss that records uses the cache.
//! G6a a package-level read through os in a dependency and in the package
//! under test is logged (guard v3). G6b a read package os never logs still
//! leaves a stale cached result, which no proof vouches for. G7 a
//! cross-compile under the overlay. G8 `go clean -testcache` where the
//! cache is expired. G9 a deferred audit expires go test's cache too. G10
//! build, vet and test share compiles, and only test binaries hold the
//! guard.

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

/// `note.txt` is read by the script and by no test: changing it makes a run
/// a miss that go test's cache can serve.
const STEP: &str =
    "cat note.txt >/dev/null && cp data.txt /tmp/quod-g3.txt && cd mod && go test ./...";

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
    env.write("note.txt", "1\n");
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

    // G2: observe mode, a miss (the script's own input changed): the
    // execution only records, with go test's cache. (An observe-mode
    // would-be hit is evidence and expires it, as G5 shows.)
    env.write("note.txt", "2\n");
    env.commit("g: a note");
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

/// G5: trust v2's evidence runs. A manifest proving itself (`--proofs 2`),
/// the key's shadow audits under the defaults, and an observe-mode
/// would-be hit each run every test fresh; between them, once the shadow
/// is over, the key replays.
#[test]
fn g5_proofs_shadow_audits_and_observe_hits_expire_go_tests() {
    if !enabled() {
        return;
    }
    let mut env = module();
    env.run_defaults = ["--warm"].map(String::from).to_vec();
    let step = "cd mod && go test ./a ./e ./d";
    let all_ran = |o: &[(String, String)]| o.len() == 3 && o.iter().all(|(_, w)| w == "ran");

    let r1 = env.run_step(step, &["--mode", "replay", "--proofs", "2"]);
    assert!(r1.executed() && r1.code == 0, "G5 miss: {}", r1.stderr);
    let r2 = env.run_step(step, &["--mode", "replay", "--proofs", "2"]);
    let o2 = outcomes(&r2.stdout);
    assert!(
        r2.executed() && all_ran(&o2),
        "G5 proof 2 runs fresh: {o2:?} {}",
        r2.stderr
    );
    for nth in [1, 2] {
        let r = env.run_step(step, &["--mode", "replay"]);
        let o = outcomes(&r.stdout);
        assert!(
            r.stderr
                .contains(&format!("shadow audit: would-be hit {nth}")),
            "G5 shadow {nth}: {}",
            r.stderr
        );
        assert!(all_ran(&o), "G5 shadow audit {nth} runs fresh: {o:?}");
    }
    let r = env.run_step(step, &["--mode", "replay"]);
    assert!(r.hit(), "G5 past the shadow, hit 3 replays: {}", r.stderr);
    let r = env.run_step(step, &["--mode", "observe"]);
    let o = outcomes(&r.stdout);
    assert!(
        r.stderr.contains("observe mode: would have replayed") && all_ran(&o),
        "G5 an observe-mode hit runs fresh: {o:?} {}",
        r.stderr
    );
    println!("G5 ok: proof 2, both shadow audits and the observe hit ran fresh");
}

/// A module of `files` (paths relative to the repo) in place of the tiny
/// one, committed.
fn small(files: &[(&str, &str)]) -> Env {
    let env = Env::new();
    for p in ["go.mod", "calc", "other"] {
        let _ = std::fs::remove_dir_all(env.repo.join(p));
        let _ = std::fs::remove_file(env.repo.join(p));
    }
    for (p, src) in files {
        env.write(p, src);
    }
    env.commit("g: a small module");
    env
}

const GO_MOD: &str = "module example.com/g\n\ngo 1.27\n";

/// The proofs-only defaults G6a and G6b run under: replay mode, two
/// proofs, no audits, so the proofs are the evidence.
fn proofs_only(env: &mut Env) {
    env.run_defaults = [
        "--warm", "--mode", "replay", "--proofs", "2", "--audit", "0",
    ]
    .map(String::from)
    .to_vec();
}

/// G6a (guard v3, the package layout): a file read through os by a
/// package-level variable is logged wherever it runs: in a dependency's
/// init (package h, read by its external test) and in the package under
/// test's own non-test file (package k, with an internal test). The guard
/// package initializes before any package that imports os, so go test
/// rechecks both files: after they change, the miss runs and fails each
/// package, never serving `(cached)`. (Under v2 the guard lived in package
/// testing, initialized after h, whose read was not logged; k's was, its
/// test files importing testing: h is the case that tells the layouts
/// apart, k the control.)
#[test]
fn g6a_a_package_level_read_in_a_dependency_and_in_the_package_is_logged() {
    if !enabled() {
        return;
    }
    let mut env = small(&[
        ("mod/go.mod", GO_MOD),
        (
            "mod/h/h.go",
            r#"package h
import ("os"; "strings")
var Conf = read()
func read() string { b, _ := os.ReadFile("testdata/conf.txt"); return strings.TrimSpace(string(b)) }
"#,
        ),
        (
            "mod/h/h_test.go",
            r#"package h_test
import ("testing"; "example.com/g/h")
func TestH(t *testing.T) { if h.Conf != "v1" { t.Fatalf("conf: %q", h.Conf) } }
"#,
        ),
        ("mod/h/testdata/conf.txt", "v1\n"),
        (
            "mod/k/k.go",
            r#"package k
import ("os"; "strings")
var Conf = read()
func read() string { b, _ := os.ReadFile("testdata/conf.txt"); return strings.TrimSpace(string(b)) }
"#,
        ),
        (
            "mod/k/k_test.go",
            r#"package k
import "testing"
func TestK(t *testing.T) { if Conf != "v1" { t.Fatalf("conf: %q", Conf) } }
"#,
        ),
        ("mod/k/testdata/conf.txt", "v1\n"),
    ]);
    proofs_only(&mut env);
    let step = "cd mod && go test ./...";
    let r1 = env.run_step(step, &[]);
    assert!(r1.executed() && r1.code == 0, "G6a first: {}", r1.stderr);
    let r2 = env.run_step(step, &[]);
    assert!(r2.executed() && r2.code == 0, "G6a proof: {}", r2.stderr);
    let r3 = env.run_step(step, &[]);
    assert!(r3.hit(), "G6a: proven, then replayed: {}", r3.stderr);
    // An unrelated change: a miss that go test's cache serves whole, the
    // reads being logged and the files unchanged.
    env.write("mod/README", "x\n");
    env.commit("g: unrelated");
    let r4 = env.run_step(step, &[]);
    let o4 = outcomes(&r4.stdout);
    assert!(
        r4.executed() && r4.code == 0,
        "G6a unrelated: {}",
        r4.stderr
    );
    assert_eq!(of(&o4, "h"), "cached", "G6a h logged and unchanged: {o4:?}");
    assert_eq!(of(&o4, "k"), "cached", "G6a k logged and unchanged: {o4:?}");
    // The files change: the miss reruns both packages, and they fail.
    env.write("mod/h/testdata/conf.txt", "v2\n");
    env.write("mod/k/testdata/conf.txt", "v2\n");
    env.commit("g: h's and k's files changed");
    let r5 = env.run_step(step, &[]);
    let o5 = outcomes(&r5.stdout);
    assert!(r5.executed() && r5.code != 0, "G6a miss: {}", r5.stderr);
    assert_eq!(
        of(&o5, "h"),
        "FAIL",
        "G6a h, read in a dependency's init: {o5:?}"
    );
    assert_eq!(
        of(&o5, "k"),
        "FAIL",
        "G6a k, read in the package's init: {o5:?}"
    );
    println!(
        "G6a ok: package-level reads in h (a dependency) and k (under test) were logged: {o5:?}"
    );
}

/// MAJOR 2, G6b: package h's variable is read from a file by h's own init
/// through syscall directly, which package os never logs, so neither the
/// guard nor go test sees it, and go test serves its cached result after
/// the file changed (as plain go test does). A proof must not vouch for
/// that: proofs run with go test's cache expired, so the stale result
/// diverges instead of replaying. (Under v2 an os.ReadFile in h's init was
/// this case too; G6a shows v3 logs it.)
#[test]
fn g6b_a_proof_never_vouches_for_a_cached_result() {
    if !enabled() {
        return;
    }
    let mut env = small(&[
        ("mod/go.mod", GO_MOD),
        (
            "mod/h/h.go",
            r#"package h
import "syscall"
var Conf = read()
func read() string {
	fd, err := syscall.Open("testdata/conf.txt", syscall.O_RDONLY, 0)
	if err != nil { return "" }
	defer syscall.Close(fd)
	var b [64]byte
	n, _ := syscall.Read(fd, b[:])
	s := string(b[:n])
	for len(s) > 0 && s[len(s)-1] == '\n' { s = s[:len(s)-1] }
	return s
}
"#,
        ),
        (
            "mod/h/h_test.go",
            r#"package h_test
import ("testing"; "example.com/g/h")
func TestH(t *testing.T) { if h.Conf != "v1" { t.Fatalf("conf: %q", h.Conf) } }
"#,
        ),
        ("mod/h/testdata/conf.txt", "v1\n"),
    ]);
    // Replay mode, two proofs, no audits: the proofs are the evidence.
    env.run_defaults = [
        "--warm", "--mode", "replay", "--proofs", "2", "--audit", "0",
    ]
    .map(String::from)
    .to_vec();
    let step = "cd mod && go test ./...";
    let r1 = env.run_step(step, &[]);
    assert!(r1.executed() && r1.code == 0, "G6 first: {}", r1.stderr);
    let r2 = env.run_step(step, &[]);
    assert!(r2.executed() && r2.code == 0, "G6 proof: {}", r2.stderr);
    let r3 = env.run_step(step, &[]);
    assert!(r3.hit(), "G6: proven, then replayed: {}", r3.stderr);

    env.write("mod/h/testdata/conf.txt", "v2\n");
    env.commit("g: h's file changed");
    // A miss: go test serves its cached result, stale, as plain go test.
    let r4 = env.run_step(step, &[]);
    assert!(r4.executed(), "G6 miss: {}", r4.stderr);
    assert_eq!(
        of(&outcomes(&r4.stdout), "h"),
        "cached",
        "G6: the guard is on and go test served h"
    );
    // Its proof runs h again, fails, and diverges; nothing replays the
    // stale result, and h fails as it does without a cache.
    let mut last = r4;
    for i in 5..=6 {
        let r = env.run_step(step, &[]);
        assert!(!r.hit(), "G6 run {i} replayed a stale result: {}", r.stderr);
        last = r;
    }
    assert_ne!(last.code, 0, "G6: {}", last.stderr);
    assert_eq!(of(&outcomes(&last.stdout), "h"), "FAIL");
    println!("G6 ok: a stale cached result diverged in its proof, never replayed");
}

/// MAJOR 3: the overlay applies to every build in a guarded step, a
/// cross-compile included; the guard's testing file is linux only, so a
/// package importing testing still builds for windows.
#[test]
fn g7_a_cross_compile_of_testing_builds_in_a_guarded_step() {
    if !enabled() {
        return;
    }
    let mut env = small(&[
        ("mod/go.mod", GO_MOD),
        (
            "mod/tu/tu.go",
            "package tu\nimport \"testing\"\nfunc Helper(t testing.TB) { t.Helper() }\n",
        ),
        (
            "mod/tu/tu_test.go",
            "package tu\nimport \"testing\"\nfunc TestTU(t *testing.T) { Helper(t) }\n",
        ),
    ]);
    env.run_defaults = ["--warm"].map(String::from).to_vec();
    let step = "cd mod && GOOS=windows go build ./... && go test ./...";
    let r1 = env.run_step(step, &[]);
    assert_eq!(r1.code, 0, "G7: {}", r1.stderr);
    // And the guard was on: after an unrelated change, go test serves tu.
    env.write("mod/README", "x\n");
    env.commit("g: unrelated");
    let r2 = env.run_step(step, &[]);
    assert_eq!(r2.code, 0, "G7: {}", r2.stderr);
    assert_eq!(of(&outcomes(&r2.stdout), "tu"), "cached", "G7");
    println!("G7 ok: GOOS=windows go build compiled package testing under the guard");
}

/// MINOR 7: an execution with go test's cache expired (here the audit)
/// lets the step's own `go clean -testcache` write the expiry file it was
/// given.
#[test]
fn g8_go_clean_testcache_works_where_the_cache_is_expired() {
    if !enabled() {
        return;
    }
    let mut env = small(&[
        ("mod/go.mod", GO_MOD),
        (
            "mod/a/a_test.go",
            "package a\nimport \"testing\"\nfunc TestA(t *testing.T) {}\n",
        ),
    ]);
    env.run_defaults = [
        "--warm", "--mode", "replay", "--proofs", "1", "--audit", "1",
    ]
    .map(String::from)
    .to_vec();
    let step = "cd mod && go clean -testcache && go test ./...";
    let r1 = env.run_step(step, &[]);
    assert!(r1.executed() && r1.code == 0, "G8 first: {}", r1.stderr);
    let r2 = env.run_step(step, &[]);
    assert!(r2.executed() && r2.code == 0, "G8 audit: {}", r2.stderr);
    assert!(!r2.stderr.contains("DIVERGENCE"), "G8: {}", r2.stderr);
    println!("G8 ok: go clean -testcache in an audit");
}

/// Trust v2, deferred audits: a decay-phase audit of a guarded warm step
/// runs later, from the recorded tree, with go test's cache expired for
/// it: every package runs, none is served from the cache the earlier
/// executions left, exactly as an inline audit (G1, G5) does.
#[test]
fn g9_a_deferred_audit_expires_go_tests_cache_too() {
    if !enabled() {
        return;
    }
    let mut env = module();
    env.run_defaults.clear();
    let repo = env.repo.to_str().unwrap().to_string();
    let run = || {
        env.quod_in(
            &env.state(),
            &[
                "run",
                "--repo",
                &repo,
                "--toolchain",
                "go",
                "--step",
                STEP,
                "--warm",
                "--mode",
                "replay",
                "--audit",
                "1",
            ],
            &[("QUOD_AUDIT_SPAWN", "0")],
        )
    };
    let r1 = run();
    assert!(r1.executed() && r1.code == 0, "G9 first: {}", r1.stderr);
    for nth in 1..=2 {
        let r = run();
        assert!(
            r.executed()
                && r.stderr
                    .contains(&format!("(shadow audit: would-be hit {nth} re-executed)")),
            "G9 shadow {nth}: {}",
            r.stderr
        );
    }
    // Hit 3: deferred; the job gets the replay and runs nothing.
    let r = run();
    assert!(
        r.hit() && r.stderr.contains("trust: audit deferred: would-be hit 3"),
        "{}",
        r.stderr
    );
    let w = env.quod_in(&env.state(), &["audit"], &[("QUOD_AUDIT_SPAWN", "0")]);
    assert_eq!(w.code, 0, "{}", w.stderr);
    let o = outcomes(&w.stdout);
    assert!(
        o.len() == PKGS.len() && o.iter().all(|(_, what)| what == "ran"),
        "G9: a deferred audit runs every test fresh: {o:?}\n{}",
        w.stderr
    );
    assert!(
        w.stderr.contains("trust: deferred audit passed") && !w.stderr.contains("DIVERGENCE"),
        "{}",
        w.stderr
    );
    println!(
        "G9 ok: the deferred audit ran all {} packages fresh",
        o.len()
    );
}

/// G10 (guard v3): `go build`, `go vet` and `go test` share compiles. The
/// overlay changes testdeps and adds the guard's package, which only test
/// mains import, so a package that imports testing (tu, a helper) compiles
/// the same with and without it. After a build and a vet step warmed the
/// cache, the test step compiles only testdeps, the guard and the test
/// mains; and the guard is in every test binary's dependencies and in no
/// non-test build's.
#[test]
fn g10_build_vet_and_test_share_compiles_and_only_test_binaries_hold_the_guard() {
    if !enabled() {
        return;
    }
    let mut env = small(&[
        ("mod/go.mod", GO_MOD),
        (
            "mod/tu/tu.go",
            "package tu\nimport \"testing\"\nfunc Helper(t testing.TB) { t.Helper() }\nfunc Two() int { return 2 }\n",
        ),
        (
            "mod/a/a.go",
            "package a\nimport \"example.com/g/tu\"\nfunc Two() int { return tu.Two() }\n",
        ),
        (
            "mod/a/a_test.go",
            "package a\nimport (\"testing\"; \"example.com/g/tu\")\nfunc TestA(t *testing.T) { tu.Helper(t); if Two() != 2 { t.Fatal(Two()) } }\n",
        ),
        ("mod/b/b.go", "package b\nfunc One() int { return 1 }\n"),
        (
            "mod/b/b_test.go",
            "package b_test\nimport (\"testing\"; \"example.com/g/b\")\nfunc TestB(t *testing.T) { if b.One() != 1 { t.Fatal() } }\n",
        ),
    ]);
    env.run_defaults = ["--warm"].map(String::from).to_vec();
    let r1 = env.run_step("cd mod && go build ./... && go vet ./...", &[]);
    assert_eq!(r1.code, 0, "G10 build and vet: {}", r1.stderr);
    // The test step, guarded; -x names every compile it runs. (No `-c`
    // word anywhere in it, `grep -c` included: the guard reads that as a
    // test binary left behind and keeps -count=1.)
    let step = "cd mod && go test -x ./... 2>/tmp/quod-g10.log >/dev/null; r=$?; \
        echo \"compiled: $(grep '/compile ' /tmp/quod-g10.log | sed -n 's/.* -p \\([^ ]*\\).*/\\1/p' | sort | tr '\\n' ' ')\"; \
        echo \"guard-in-build: $(go list -deps ./... | grep 0quod/guard | wc -l)\"; \
        echo \"guard-in-test: $(go list -deps -test ./... | grep 0quod/guard | wc -l)\"; \
        exit $r";
    let r2 = env.run_step(step, &[]);
    assert_eq!(r2.code, 0, "G10 test: {}", r2.stderr);
    let out = String::from_utf8_lossy(&r2.stdout);
    let line = |p: &str| {
        out.lines()
            .find_map(|l| l.strip_prefix(p))
            .unwrap_or_else(|| panic!("G10: no {p:?} line in {out}"))
            .trim()
            .to_string()
    };
    let compiled: Vec<String> = line("compiled:")
        .split_whitespace()
        .map(String::from)
        .collect();
    // Not tu (imports testing), not b (a plain package whose test is
    // external), not testing itself: build and vet compiled those, and go
    // test took them from the cache. What go test compiles is the test
    // mains, testdeps, the guard, testdeps' own standard dependencies
    // (internal/fuzz, runtime/pprof, ...), and the test variants (a with
    // its _test.go, b_test): stock go1.27 recompiles those two after vet
    // as well (checked 2026-11-02 without the overlay), the guard aside.
    for p in ["example.com/g/tu", "example.com/g/b", "testing"] {
        assert!(
            !compiled.iter().any(|c| c == p),
            "G10: go test compiled {p}, which build or vet had compiled already: {compiled:?}"
        );
    }
    for p in ["main", "testing/internal/testdeps", "0quod/guard"] {
        assert!(
            compiled.iter().any(|c| c == p),
            "G10: {p} not compiled by go test: {compiled:?}"
        );
    }
    assert_eq!(line("guard-in-build:"), "0", "G10: {out}");
    assert_eq!(line("guard-in-test:"), "1", "G10: {out}");
    println!("G10 ok: go test compiled only {compiled:?}; the guard is in test builds only");
}
