//! The vitest recorder's package.json rule (src/vitest/record.mjs), run in
//! node: a read by vite's resolver is keyed field-level (`pj:`); a read by
//! anything else in vitest's main process, here a plugin that would inject
//! `version` into code, keeps the whole file (`f:`), so a version bump
//! re-runs what that plugin touched. Skipped when there is no node.

use std::fs;
use std::path::Path;
use std::process::Command;

fn write(p: &Path, s: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, s).unwrap();
}

#[test]
fn a_plugin_reading_package_json_keeps_it_whole() {
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("no node: skipped");
        return;
    }
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    write(
        &root.join("package.json"),
        r#"{"name":"x","version":"1.0.0"}"#,
    );
    let reader = "const fs = require('node:fs');\nmodule.exports = (p) => JSON.parse(fs.readFileSync(p, 'utf8'));\n";
    write(&root.join("node_modules/vite/dist/resolve.cjs"), reader);
    write(
        &root.join("node_modules/vite-plugin-version/index.cjs"),
        reader,
    );
    let record = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/vitest/record.mjs");
    let script = format!(
        r#"
import {{ createRecorder, patchFs }} from {record:?}
import {{ createRequire }} from 'node:module'
const req = createRequire({pj:?})
const r = createRecorder({{ stateDir: '/nonexistent-state', mode: 'main' }})
patchFs(r)
const pj = {pj:?}
req('./node_modules/vite/dist/resolve.cjs')(pj)
const resolver = [...r.keys]
r.keys.clear()
req('./node_modules/vite-plugin-version/index.cjs')(pj)
const pjOnly = (ks) => ks.filter((k) => k.endsWith('/package.json'))
console.log(JSON.stringify({{ resolver: pjOnly(resolver), plugin: pjOnly([...r.keys]) }}))
"#,
        record = record.display().to_string(),
        pj = root.join("package.json").display().to_string(),
    );
    let s = root.join("t.mjs");
    fs::write(&s, script).unwrap();
    let out = Command::new("node").arg(&s).output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let pj = root.join("package.json").display().to_string();
    assert_eq!(v["resolver"], serde_json::json!([format!("pj:{pj}")]));
    assert_eq!(v["plugin"], serde_json::json!([format!("f:{pj}")]));
}

#[test]
fn the_preloaded_tostring_sees_a_reference_cached_before_setup() {
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("no node: skipped");
        return;
    }
    let t = tempfile::tempdir().unwrap();
    let preload = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/vitest/preload.cjs");
    // A library caches the method at load (before Quod's setup file), the
    // setup file resets the flag, a test then formats a function's source.
    let script = r#"
const fnToString = Function.prototype.toString
const state = globalThis.__quod_toString
state.used = false
const name = fnToString.call(() => false)
const native = Function.prototype.toString.call(Function.prototype.toString)
console.log(JSON.stringify({ used: state.used, name, native }))
"#;
    let s = t.path().join("t.cjs");
    fs::write(&s, script).unwrap();
    let out = Command::new("node")
        .arg("--require")
        .arg(&preload)
        .arg(&s)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["used"], true);
    assert_eq!(v["name"], "() => false");
    assert_eq!(v["native"], "function toString() { [native code] }");
}

#[test]
fn a_read_through_a_workspace_link_is_keyed_at_the_real_path() {
    // npm (and pnpm, yarn) workspaces link a workspace package into
    // node_modules. Its package.json says nothing about its other files, so
    // a read through the link is keyed as a read of the real file (plus the
    // link); an installed package stays one `p:` key.
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("no node: skipped");
        return;
    }
    let t = tempfile::tempdir().unwrap();
    let root = t.path().canonicalize().unwrap();
    write(&root.join("packages/b/package.json"), r#"{"name":"@ws/b"}"#);
    write(&root.join("packages/b/data/x.json"), "{}");
    write(&root.join("node_modules/c/package.json"), r#"{"name":"c"}"#);
    write(&root.join("node_modules/c/index.js"), "");
    fs::create_dir_all(root.join("node_modules/@ws")).unwrap();
    std::os::unix::fs::symlink("../../packages/b", root.join("node_modules/@ws/b")).unwrap();
    let record = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/vitest/record.mjs");
    let script = format!(
        r#"
import {{ createRecorder, patchFs }} from {record:?}
import fs from 'node:fs'
const r = createRecorder({{ stateDir: '/nonexistent-state', mode: 'worker' }})
patchFs(r)
fs.readFileSync({link:?})
fs.readdirSync({linkdir:?})
fs.readFileSync({inst:?})
console.log(JSON.stringify([...r.keys].sort()))
"#,
        record = record.display().to_string(),
        link = root
            .join("node_modules/@ws/b/data/x.json")
            .display()
            .to_string(),
        linkdir = root.join("node_modules/@ws/b/data").display().to_string(),
        inst = root.join("node_modules/c/index.js").display().to_string(),
    );
    let s = root.join("t.mjs");
    fs::write(&s, script).unwrap();
    let out = Command::new("node").arg(&s).output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Vec<String> = serde_json::from_slice(&out.stdout).unwrap();
    let r = root.display().to_string();
    assert_eq!(
        v,
        vec![
            format!("d:{r}/packages/b/data"),
            format!("e:{r}/node_modules/@ws/b"),
            format!("f:{r}/packages/b/data/x.json"),
            format!("p:{r}/node_modules/c"),
        ]
    );
}

#[test]
fn the_env_proxy_takes_assignments() {
    // vi.stubEnv assigns process.env.X; through a proxy without a `set`
    // trap that became a partial defineProperty, which process.env refuses.
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("no node: skipped");
        return;
    }
    let t = tempfile::tempdir().unwrap();
    let record = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/vitest/record.mjs");
    let script = format!(
        r#"
import {{ createRecorder, patchEnv }} from {record:?}
const r = createRecorder({{ stateDir: '/nonexistent-state', mode: 'worker' }})
process.env.QUOD_T = 'a'
patchEnv(r)
process.env.QUOD_T = 'b'
process.env.QUOD_U = 'c'
delete process.env.QUOD_U
console.log(JSON.stringify([process.env.QUOD_T, process.env.QUOD_U ?? null]))
"#,
        record = record.display().to_string(),
    );
    let s = t.path().join("t.mjs");
    fs::write(&s, script).unwrap();
    let out = Command::new("node").arg(&s).output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v, serde_json::json!(["b", null]));
}
