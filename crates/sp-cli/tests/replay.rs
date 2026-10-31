//! T10: the hit path.
mod common;
use common::*;

#[test]
fn second_run_on_same_tree_replays_identically() {
    if !enabled() {
        return;
    }
    let env = Env::new();
    let first = env.run_step("go test -v ./... && echo to-stderr >&2", &[]);
    assert!(first.executed() && first.code == 0, "{}", first.stderr);
    let second = env.run_step("go test -v ./... && echo to-stderr >&2", &[]);
    assert!(second.hit(), "{}", second.stderr);
    assert_eq!(second.code, first.code);
    assert_eq!(
        second.stdout, first.stdout,
        "replayed stdout must be byte-identical"
    );
    assert!(second.stderr.ends_with("to-stderr\n"));
    assert!(second.stderr.contains("inputs matched"));

    // A failing step replays its failure.
    let f1 = env.run_step("echo nope; exit 4", &[]);
    let f2 = env.run_step("echo nope; exit 4", &[]);
    assert!(f2.hit());
    assert_eq!((f1.code, f2.code), (4, 4));
    assert_eq!(f2.stdout, b"nope\n");
}

#[test]
fn outputs_are_written_on_execute_and_replay() {
    if !enabled() {
        return;
    }
    let env = Env::new();
    // Also one compared and one volatile path under HOME: --outputs writes
    // what the trust layer compares, never a path it does not.
    let step = r#"mkdir -p out && echo built > out/artifact.txt && rm README.md && mkdir -p "$HOME/.cache/gh" && echo v > "$HOME/.cache/gh/entry" && echo keep > "$HOME/tok""#;
    for (i, want_hit) in [(0, false), (1, true)] {
        let dir = env.tmp.path().join(format!("outputs{i}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("README.md"), "stale").unwrap();
        let o = env.run_step(step, &["--outputs", dir.to_str().unwrap()]);
        assert_eq!(o.hit(), want_hit, "{}", o.stderr);
        assert_eq!(
            std::fs::read_to_string(dir.join("out/artifact.txt")).unwrap(),
            "built\n"
        );
        assert!(!dir.join("README.md").exists(), "deletion applied");
        assert_eq!(
            std::fs::read_to_string(dir.join(".stillpass/home/tok")).unwrap(),
            "keep\n"
        );
        assert!(
            !dir.join(".stillpass/home/.cache/gh").exists(),
            "an uncompared path is never written out"
        );
        assert!(
            o.stderr
                .contains("not compared, and not written by --outputs: .stillpass/home/.cache/gh"),
            "the banner names it: {}",
            o.stderr
        );
    }
    assert!(
        env.repo.join("README.md").exists(),
        "the repo itself is never written"
    );
}
