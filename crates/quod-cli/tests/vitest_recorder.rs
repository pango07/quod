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

#[test]
fn reads_under_a_directory_the_file_made_outside_the_workspace_are_not_keyed() {
    // A test that makes a temp directory (mkdtemp under the OS temp dir,
    // a random path every run) and reads inside it reads its own output:
    // keying those reads made the file run every time (vuejs/router's
    // RoutesFolderWatcher.spec.ts). A path that existed before, a check
    // made before the directory existed or after it was removed, and
    // anything under a directory made inside the workspace stay keyed.
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("no node: skipped");
        return;
    }
    let t = tempfile::tempdir().unwrap();
    let base = t.path().canonicalize().unwrap();
    let ws = base.join("ws");
    write(&ws.join("src/a.txt"), "a");
    write(&base.join("pre/x.txt"), "x");
    let record = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/vitest/record.mjs");
    let script = format!(
        r#"
import {{ createRecorder, patchFs }} from {record:?}
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
const r = createRecorder({{ stateDir: '/nonexistent-state', mode: 'worker', wsRoot: {ws:?} + '/' }})
patchFs(r)
const made = []
// sync mkdtemp, a recursive mkdir and files inside it
const a = fs.mkdtempSync(path.join(os.tmpdir(), 'quod-own-'))
made.push(a)
fs.mkdirSync(path.join(a, 'test-0/src/pages'), {{ recursive: true }})
fs.writeFileSync(path.join(a, 'test-0/src/pages/index.vue'), 'x')
fs.readdirSync(path.join(a, 'test-0/src/pages'))
fs.readFileSync(path.join(a, 'test-0/src/pages/index.vue'))
fs.statSync(path.join(a, 'test-0/src'))
fs.existsSync(path.join(a, 'nope'))
fs.readdirSync(a, {{ recursive: true }})
fs.watch(a).close()
// promise and callback mkdtemp
const b = await fs.promises.mkdtemp(path.join(os.tmpdir(), 'quod-own-'))
made.push(b)
await fs.promises.readdir(b)
// its teardown removing it (rm reads the directory) is its own too
fs.rmSync(b, {{ recursive: true }})
const c = await new Promise((res, rej) => fs.mkdtemp(path.join(os.tmpdir(), 'quod-own-'), (e, d) => (e ? rej(e) : res(d))))
made.push(c)
fs.readdirSync(c)
// a fixed path: checked before it existed (kept), made, listed (not
// kept), removed, checked again (kept)
const fixed = path.join(os.tmpdir(), `quod-fixed-${{process.pid}}-${{Date.now()}}`)
fs.existsSync(fixed)
fs.mkdirSync(fixed)
made.push(fixed)
fs.readdirSync(fixed)
fs.rmSync(fixed, {{ recursive: true }})
fs.existsSync(path.join(fixed, 'after'))
// a path that existed before, outside the workspace; a directory made
// inside the workspace
fs.readFileSync({pre:?})
fs.mkdirSync({wsdir:?})
fs.readdirSync({wsdir:?})
const out = {{ keys: [...r.keys].sort(), watches: [...r.watches], made }}
for (const d of [a, c]) fs.rmSync(d, {{ recursive: true }})
console.log(JSON.stringify(out))
"#,
        record = record.display().to_string(),
        ws = ws.display().to_string(),
        pre = base.join("pre/x.txt").display().to_string(),
        wsdir = ws.join("src/made").display().to_string(),
    );
    let s = base.join("t.mjs");
    fs::write(&s, script).unwrap();
    let out = Command::new("node").arg(&s).output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let keys: Vec<String> = serde_json::from_value(v["keys"].clone()).unwrap();
    let made: Vec<String> = serde_json::from_value(v["made"].clone()).unwrap();
    for m in &made[..3] {
        assert!(
            !keys.iter().any(|k| k.contains(m.as_str())),
            "{m} is the file's own output: {keys:?}"
        );
    }
    assert_eq!(v["watches"], serde_json::json!([]));
    let fixed = &made[3];
    let kept: Vec<&String> = keys.iter().filter(|k| k.contains("quod-fixed-")).collect();
    assert_eq!(
        kept,
        vec![&format!("e:{fixed}"), &format!("e:{fixed}/after")],
        "before it was made and after it was removed it is not the file's"
    );
    let b = base.display().to_string();
    assert!(keys.contains(&format!("f:{b}/pre/x.txt")), "{keys:?}");
    assert!(keys.contains(&format!("d:{b}/ws/src/made")), "{keys:?}");
}

/// Runs `body` (an ES module's code, after `r` is a main-process recorder
/// with fs and env patched) in `root`; prints what it prints.
fn main_recorder(root: &Path, body: &str) -> Option<serde_json::Value> {
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("no node: skipped");
        return None;
    }
    let record = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/vitest/record.mjs");
    let script = format!(
        r#"
import {{ createRecorder, patchEnv, patchFs, settledKeys }} from {record:?}
import {{ createRequire }} from 'node:module'
const req = createRequire(import.meta.url)
const r = createRecorder({{ stateDir: '/nonexistent-state', mode: 'main' }})
patchFs(r)
patchEnv(r)
{body}
"#,
        record = record.display().to_string(),
    );
    let s = root.join("t.mjs");
    fs::write(&s, script).unwrap();
    let out = Command::new("node")
        .arg(&s)
        .current_dir(root)
        .env("GITHUB_HEAD_REF", "feature/x")
        .env("GITHUB_EVENT_NAME", "pull_request")
        .env("npm_package_version", "1.2.3")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    Some(serde_json::from_slice(&out.stdout).unwrap())
}

#[test]
fn the_environment_esbuild_copies_for_its_service_is_not_every_files_input() {
    // esbuild's JS API spawns its Go service with node's copy of the whole
    // environment (node lists and reads every variable). That copy keys
    // what the child behaves on (envtool:, Go's and esbuild's settings),
    // not the branch, the event or the package version. Any other program's
    // child keeps the whole environment keyed.
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    let spawner = "const cp = require('node:child_process');\nmodule.exports = () => cp.spawnSync(process.execPath, ['-e', '0']);\n";
    write(&root.join("node_modules/esbuild/lib/main.js"), spawner);
    write(&root.join("node_modules/some-plugin/index.js"), spawner);
    let Some(v) = main_recorder(
        root,
        r#"
const env = (ks) => [...ks].filter((k) => k.startsWith('env'))
req('./node_modules/esbuild/lib/main.js')()
const esbuild = env(r.keys)
r.keys.clear()
req('./node_modules/some-plugin/index.js')()
console.log(JSON.stringify({ esbuild, plugin: env(r.keys), child: r.envChild.size }))
"#,
    ) else {
        return;
    };
    let has = |side: &str, k: &str| v[side].as_array().unwrap().iter().any(|x| x == k);
    assert!(has("esbuild", "envtool:"), "{v}");
    assert!(has("esbuild", "env:GOMAXPROCS"), "{v}");
    for k in [
        "envall:",
        "env:GITHUB_HEAD_REF",
        "env:GITHUB_EVENT_NAME",
        "env:npm_package_version",
    ] {
        assert!(!has("esbuild", k), "{k} keyed by esbuild's copy: {v}");
        assert!(
            has("plugin", k),
            "{k} not keyed by another child's copy: {v}"
        );
    }
    assert_eq!(v["child"], 1, "{v}");
}

#[test]
fn a_package_json_parsed_straight_is_keyed_by_the_fields_read() {
    // convex-helpers' config: JSON.parse(readFileSync('package.json')) and
    // the dependency names. Keyed by its names and the fields read (pjf:),
    // so a version bump changes nothing; read into a variable first (the
    // text could be used again), it stays whole (f:).
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    write(
        &root.join("package.json"),
        r#"{"name":"x","version":"1.0.0","dependencies":{"zod-4.1":"npm:zod@4.1"}}"#,
    );
    write(
        &root.join("cfg/straight.mjs"),
        "import { readFileSync } from 'node:fs'\n\
         const pkg = JSON.parse(\n  readFileSync(new URL('../package.json', import.meta.url), 'utf8'),\n)\n\
         export const names = Object.keys({ ...pkg.dependencies })\n\
         export const clone = structuredClone({ deps: pkg.dependencies })\n",
    );
    write(
        &root.join("cfg/variable.mjs"),
        "import { readFileSync } from 'node:fs'\n\
         const raw = readFileSync(new URL('../package.json', import.meta.url), 'utf8')\n\
         export const pkg = JSON.parse(raw)\n",
    );
    let Some(v) = main_recorder(
        root,
        r#"
const pj = (ks) => [...ks].filter((k) => k.includes('package.json')).map((k) => k.replace(process.cwd() + '/', ''))
const m = await import('./cfg/straight.mjs')
const straight = pj(settledKeys(r))
r.keys.clear(); r.pjReads.clear(); r.pjParsed.clear(); r.pjUse.clear()
await import('./cfg/variable.mjs')
console.log(JSON.stringify({ straight, variable: pj(settledKeys(r)), names: m.names }))
"#,
    ) else {
        return;
    };
    assert_eq!(
        v["straight"],
        serde_json::json!(["pjf:package.json\u{0}[\"dependencies\"]"]),
        "{v}"
    );
    assert_eq!(v["variable"], serde_json::json!(["f:package.json"]), "{v}");
    assert_eq!(v["names"], serde_json::json!(["zod-4.1"]), "{v}");
}

#[test]
fn tsconfcks_walk_is_keyed_by_the_json_files_under_its_start() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    write(
        &root.join("node_modules/tsconfck/dist/index.js"),
        "const fs = require('node:fs');\nconst ls = (d) => new Promise((res) => fs.readdir(d, { withFileTypes: true }, () => res()));\nmodule.exports = async (d) => { await ls(d); await ls(d + '/x') };\n",
    );
    write(
        &root.join("plain.js"),
        "const fs = require('node:fs');\nmodule.exports = (d) => fs.readdirSync(d);\n",
    );
    let Some(v) = main_recorder(
        root,
        r#"
await req('./node_modules/tsconfck/dist/index.js')(process.cwd() + '/node_modules')
req('./plain.js')(process.cwd() + '/node_modules')
console.log(JSON.stringify([...r.keys].filter((k) => /^r?[dj]+:/.test(k)).map((k) => k.replace(process.cwd() + '/', ''))))
"#,
    ) else {
        return;
    };
    assert_eq!(
        v,
        serde_json::json!(["rj:node_modules", "d:node_modules"]),
        "{v}"
    );
}

#[test]
fn a_commonjs_modules_top_level_read_is_the_modules() {
    // V8 names a CommonJS module's top level `Object.<anonymous> (file)`.
    // Taken for a frame without a file, a library's read at load, required
    // from vitest, counted as vitest's own and was not keyed at all.
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    write(
        &root.join("node_modules/some-lib/index.js"),
        "module.exports = { copy: { ...process.env }, foo: process.env.QUOD_T_FOO };\n",
    );
    write(
        &root.join("node_modules/vitest/dist/load.js"),
        "module.exports = (req, done) => setImmediate(() => {\n  req('some-lib');\n  done();\n});\n",
    );
    let Some(v) = main_recorder(
        root,
        r#"
// required from vitest's own callback: no other frame on the stack
const load = req('./node_modules/vitest/dist/load.js')
await new Promise((done) => load(createRequire(process.cwd() + '/node_modules/vitest/dist/load.js'), done))
console.log(JSON.stringify([...r.keys].filter((k) => k.startsWith('env'))))
"#,
    ) else {
        return;
    };
    let has = |k: &str| v.as_array().unwrap().iter().any(|x| x == k);
    assert!(has("env:QUOD_T_FOO"), "{v}");
    assert!(has("envall:"), "{v}");
    assert!(has("env:GITHUB_HEAD_REF"), "{v}");
}
