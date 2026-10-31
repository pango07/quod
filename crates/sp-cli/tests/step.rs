//! The runner API (`stillpass step`): one step in a pinned rootfs, from a
//! JSON request to a JSON response, against a tiny rootfs built from this
//! host's binaries.

mod common;

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use common::{enabled, mini_rootfs};
use serde_json::{Value, json};

struct Api {
    tmp: tempfile::TempDir,
}

struct Reply {
    code: i32,
    stdout: String,
    stderr: String,
    response: Option<Value>,
}

impl Api {
    fn state(&self) -> PathBuf {
        self.tmp.path().join("state")
    }

    fn stillpass(&self, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_stillpass"))
            .arg("--state")
            .arg(self.state())
            .args(args)
            .env_remove("STILLPASS_ROOTFS")
            .output()
            .unwrap()
    }

    fn step(&self, req: &Value) -> Reply {
        let rq = self.tmp.path().join("request.json");
        let rs = self.tmp.path().join("response.json");
        let _ = fs::remove_file(&rs);
        fs::write(&rq, serde_json::to_vec(req).unwrap()).unwrap();
        let out = self.stillpass(&[
            "step",
            "--request",
            rq.to_str().unwrap(),
            "--response",
            rs.to_str().unwrap(),
        ]);
        let r = Reply {
            code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into(),
            stderr: String::from_utf8_lossy(&out.stderr).into(),
            response: fs::read(&rs)
                .ok()
                .map(|b| serde_json::from_slice(&b).unwrap()),
        };
        eprintln!("$ stillpass step: exit {}\n{}", r.code, r.stderr);
        r
    }
}

fn paths(entries: &Value) -> Vec<String> {
    entries
        .as_array()
        .unwrap()
        .iter()
        .map(|e| format!("{} {}", e["k"].as_str().unwrap(), e["p"].as_str().unwrap()))
        .collect()
}

#[test]
fn a_runner_runs_one_step_and_gets_result_manifest_and_outputs() {
    if !enabled() {
        return;
    }
    let api = Api {
        tmp: tempfile::tempdir().unwrap(),
    };
    let root = api.tmp.path().join("rootfs");
    mini_rootfs(&root, &["cat", "mkdir"]);
    let out = api.stillpass(&["rootfs", "import", root.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let image = String::from_utf8(out.stdout).unwrap().trim().to_string();

    let ws = api.tmp.path().join("ws");
    fs::create_dir_all(&ws).unwrap();
    fs::write(ws.join("in.txt"), "in\n").unwrap();
    fs::write(ws.join("unread.txt"), "never read\n").unwrap();
    let tools = api.tmp.path().join("tools");
    fs::create_dir_all(&tools).unwrap();
    fs::write(tools.join("t"), "v1\n").unwrap();
    let script = "cat in.txt; cat /opt/tools/t; echo out > o; mkdir -p /var/x; echo r > /var/x/r; echo t > /tmp/t; cat /etc/os-release >&2";
    let req = |lookup: bool| {
        json!({
            "image": image,
            "workspace": {"dir": ws},
            "cmd": ["/bin/sh", "-e", "-c", script],
            "cwd": "/workspace",
            "env": [["PATH", "/usr/bin:/bin"], ["LANG", "C.UTF-8"]],
            "mounts": [{"dst": "/opt/tools", "dir": tools}],
            "lookup": lookup,
        })
    };

    // First call: executes, records, returns everything.
    let a = api.step(&req(true));
    assert_eq!(a.code, 0, "{}", a.stderr);
    assert_eq!(a.stdout, "in\nv1\n", "the step's stdout streams through");
    assert!(a.stderr.contains("ID=stillpass-mini"), "{}", a.stderr);
    let r = a.response.expect("a response");
    assert_eq!(
        (r["replayed"].as_bool(), r["recorded"].as_bool()),
        (Some(false), Some(true))
    );
    let key = &r["key"];
    assert_eq!(key["image"]["kind"], "rootfs");
    assert_eq!(key["image"]["digest"], image.as_str());
    assert_eq!(key["image"]["mounts"][0][0], "/opt/tools");
    assert!(key["image"]["platform"]["cpus"].as_u64().unwrap() >= 1);
    let manifest = paths(&r["manifest"]["entries"]);
    assert!(manifest.contains(&"f in.txt".to_string()), "{manifest:?}");
    assert!(
        !manifest
            .iter()
            .any(|p| p.contains("unread") || p.contains("tools")),
        "only what the step read in the workspace: {manifest:?}"
    );
    assert_eq!(paths(&r["result"]["outputs"]), ["f o"]);
    let root_outputs = paths(&r["result"]["root_outputs"]);
    assert!(
        !root_outputs.iter().any(|p| p.contains(" tmp")),
        "/tmp is scratch: {root_outputs:?}"
    );
    for want in ["d var/x", "f var/x/r", "d opt/tools"] {
        assert!(
            root_outputs.contains(&want.to_string()),
            "{want}: {root_outputs:?}"
        );
    }
    // Every hash in the result resolves in the store.
    let blob = |h: &Value| {
        let h = h.as_str().unwrap().trim_start_matches("b3:");
        fs::read_to_string(api.state().join("cas").join(&h[..2]).join(h)).unwrap()
    };
    assert_eq!(blob(&r["result"]["stdout"]), "in\nv1\n");
    let r_out = r["result"]["root_outputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["p"] == "var/x/r")
        .unwrap();
    assert_eq!(blob(&r_out["h"]), "r\n");

    // Same request: replayed, same streams, no execution.
    let b = api.step(&req(true));
    let rb = b.response.expect("a response");
    assert_eq!(rb["replayed"], true, "{}", b.stderr);
    assert_eq!((b.code, b.stdout.as_str()), (0, "in\nv1\n"));
    assert_eq!(rb["step_key"], r["step_key"]);
    assert_eq!(rb["result"]["run_id"], r["result"]["run_id"]);
    // A file the step never read changes nothing.
    fs::write(ws.join("unread.txt"), "changed\n").unwrap();
    assert_eq!(api.step(&req(true)).response.unwrap()["replayed"], true);
    // lookup: false always executes.
    let c = api.step(&req(false)).response.unwrap();
    assert_eq!(c["replayed"], false);
    assert_ne!(c["result"]["run_id"], r["result"]["run_id"]);

    // The mount is pinned by content: other content is another key.
    fs::write(tools.join("t"), "v2\n").unwrap();
    let d = api.step(&req(true));
    let rd = d.response.unwrap();
    assert_eq!(
        (rd["replayed"].as_bool(), d.stdout.as_str()),
        (Some(false), "in\nv2\n")
    );
    assert_ne!(rd["step_key"], r["step_key"]);
    // A workspace input changed: same key, the proof no longer matches.
    fs::write(ws.join("in.txt"), "in2\n").unwrap();
    let e = api.step(&req(true));
    let re = e.response.unwrap();
    assert_eq!(
        (re["replayed"].as_bool(), e.stdout.as_str()),
        (Some(false), "in2\nv2\n")
    );
    assert_eq!(re["step_key"], rd["step_key"]);

    // A step killed by a signal is returned, never recorded, never replayed.
    let mut killed = req(true);
    killed["cmd"] = json!(["/bin/sh", "-e", "-c", "cat in.txt; kill -9 $$"]);
    for _ in 0..2 {
        let k = api.step(&killed);
        let rk = k.response.expect("a response");
        assert_eq!(
            (
                rk["replayed"].as_bool(),
                rk["recorded"].as_bool(),
                rk["signal"].as_i64()
            ),
            (Some(false), Some(false), Some(9)),
            "{}",
            k.stderr
        );
        assert_eq!(k.code, 137);
    }

    // Refused before anything runs: a mount over the observed workspace,
    // an unknown field, an unknown image. No response, exit 2.
    let mut bad = req(true);
    bad["mounts"][0]["dst"] = json!("/workspace/tools");
    let f = api.step(&bad);
    assert!(
        f.code == 2 && f.response.is_none() && f.stderr.contains("overlaps an observed root"),
        "{}",
        f.stderr
    );
    let mut bad = req(true);
    bad["warm"] = json!(true);
    assert_eq!(api.step(&bad).code, 2);
    let mut bad = req(true);
    bad["image"] = json!(format!("b3:{}", "0".repeat(64)));
    let g = api.step(&bad);
    assert!(
        g.code == 2 && g.stderr.contains("not in the store"),
        "{}",
        g.stderr
    );
}

/// A rootfs-mode step through `stillpass run` with the tiny rootfs: the
/// step sees only the rootfs, may write anywhere (recorded as root
/// outputs), and the host's userland is invisible.
#[test]
fn run_in_a_mini_rootfs_sees_no_host_and_records_writes_anywhere() {
    if !enabled() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("rootfs");
    mini_rootfs(&root, &["cat", "ls", "mkdir", "rm", "id", "mkfifo"]);
    let state = tmp.path().join("state");
    let sp = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_stillpass"))
            .arg("--state")
            .arg(&state)
            .args(args)
            .env_remove("STILLPASS_ROOTFS")
            .output()
            .unwrap()
    };
    let out = sp(&["rootfs", "import", root.to_str().unwrap()]);
    assert!(out.status.success());
    let image = String::from_utf8(out.stdout).unwrap().trim().to_string();
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    fs::write(repo.join("a.txt"), "a\n").unwrap();
    let step = "ls -a /; cat /etc/os-release; id -u; \
        for p in /usr/local/go /etc/fuse.conf /src /work /sys/kernel /.complete /.dockerenv; do test -e $p && echo LEAK $p; done; \
        echo x > /opt/new; mkdir /home2; echo y > /home2/f; echo z > /tmp/z; rm /etc/os-release; echo w > a.out; \
        rm /opt/replace-me; mkfifo /opt/replace-me; mkfifo /opt/newfifo; true";
    let run = |extra: &[&str]| {
        let mut args = vec![
            "run",
            "--repo",
            repo.to_str().unwrap(),
            "--rootfs",
            &image,
            "--step",
            step,
        ];
        args.extend_from_slice(extra);
        sp(&args)
    };
    let o = run(&[]);
    let stdout = String::from_utf8_lossy(&o.stdout);
    let stderr = String::from_utf8_lossy(&o.stderr);
    assert_eq!(o.status.code(), Some(0), "{stdout}\n{stderr}");
    assert!(!stdout.contains("LEAK"), "{stdout}");
    assert!(stdout.contains("ID=stillpass-mini\n1001\n"), "{stdout}");
    let listing: Vec<&str> = stdout
        .lines()
        .take_while(|l| !l.starts_with("ID="))
        .collect();
    for top in ["dev", "etc", "opt", "proc", "sys", "tmp", "workspace"] {
        assert!(listing.contains(&top), "{top} not in / : {listing:?}");
    }
    assert!(stderr.contains("image rootfs b3:"), "{stderr}");
    let result = Command::new("sqlite3")
        .arg(state.join("ledger.sqlite"))
        .arg("SELECT json FROM results")
        .output()
        .unwrap();
    let result: Value = serde_json::from_slice(&result.stdout).unwrap();
    let ro = paths(&result["root_outputs"]);
    assert!(
        !ro.iter().any(|p| p.contains(" tmp")),
        "/tmp is scratch: {ro:?}"
    );
    assert!(
        !ro.iter().any(|p| p.contains("newfifo")) && !ro.contains(&"f opt/replace-me".to_string()),
        "sockets and fifos are not outputs: {ro:?}"
    );
    for want in [
        "f opt/new",
        "d home2",
        "f home2/f",
        "x etc/os-release",
        // A fifo replaced an image file: not an output, but the file's
        // deletion is.
        "x opt/replace-me",
    ] {
        assert!(ro.contains(&want.to_string()), "{want}: {ro:?}");
    }
    assert!(
        !ro.iter().any(|p| p.contains("stillpass")
            || p.starts_with("f etc/passwd")
            || p.starts_with("d workspace")
            || p.starts_with("d dev")),
        "nothing of the sandbox's own: {ro:?}"
    );
    assert_eq!(paths(&result["outputs"]), ["f a.out"]);
    // Same tree, same rootfs: replayed. The snapshot was not changed by the
    // step's writes (they went to its overlay).
    let h = run(&[]);
    assert!(String::from_utf8_lossy(&h.stderr).contains("STILLPASS  REPLAYED"));
    assert_eq!(h.stdout, o.stdout);
    let o2 = run(&["--env", "AGAIN=1"]);
    assert!(
        String::from_utf8_lossy(&o2.stdout).contains("ID=stillpass-mini"),
        "the rootfs is pristine again"
    );
}
