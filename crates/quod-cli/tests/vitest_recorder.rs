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
fn stat_readers_keep_metadata_at_the_actual_path() {
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("no node: skipped");
        return;
    }
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    let input = root.join("input");
    let dependency = root.join("node_modules/pkg/data");
    write(&input, "same bytes");
    write(&dependency, "installed bytes");
    write(
        &root.join("node_modules/pkg/package.json"),
        r#"{"name":"pkg"}"#,
    );
    let record = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/vitest/record.mjs");
    let script = format!(
        r#"
import fs from 'node:fs';
import assert from 'node:assert/strict';
import {{ createRecorder, patchFs }} from {record:?};
const r = createRecorder({{stateDir:'/not-state',mode:'worker'}});
const input = {input:?}, dependency = {dependency:?};
const before = fs.statSync(input);
patchFs(r);
assert.equal(fs.statSync(input).mtimeMs, before.mtimeMs);
assert.equal((await fs.promises.stat(input)).mtimeMs, before.mtimeMs);
await new Promise((resolve,reject)=>fs.stat(input,(e,s)=>{{
  if(e) return reject(e); assert.equal(s.mtimeMs,before.mtimeMs); resolve();
}}));
fs.lstatSync(input);
await fs.promises.lstat(dependency);
fs.statSync(dependency, {{bigint:true}});
r.on=false;
console.log(JSON.stringify([...r.keys].sort()));
"#,
        record = record.display().to_string(),
        input = input.display().to_string(),
        dependency = dependency.display().to_string()
    );
    let script_path = root.join("probe.mjs");
    fs::write(&script_path, script).unwrap();
    let out = Command::new("node").arg(script_path).output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let keys: Vec<String> = serde_json::from_slice(&out.stdout).unwrap();
    for (kind, p) in [
        ("sm", &input),
        ("sl", &input),
        ("sm", &dependency),
        ("sl", &dependency),
    ] {
        assert!(
            keys.contains(&format!("{kind}:{}", p.display())),
            "{keys:?}"
        );
    }
    assert!(
        !keys.iter().any(|k| k.starts_with("p:")),
        "metadata must not collapse to a package identity"
    );
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
fn reads_under_a_directory_the_file_made_outside_the_workspace_are_keyed() {
    // Creation alone cannot prove that a later observation is independent:
    // metadata can vary, and links or imported files can reach other inputs.
    // Preserve every read and watch, including before creation, after removal,
    // and inside or outside the workspace.
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
// teardown must not erase earlier directory reads
fs.rmSync(b, {{ recursive: true }})
const c = await new Promise((res, rej) => fs.mkdtemp(path.join(os.tmpdir(), 'quod-own-'), (e, d) => (e ? rej(e) : res(d))))
made.push(c)
fs.readdirSync(c)
// a fixed path: checks before creation, a listing after creation, and
// checks after removal all remain keyed
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
    for (kind, p) in [
        ("f", format!("{}/test-0/src/pages/index.vue", made[0])),
        ("sm", format!("{}/test-0/src", made[0])),
        ("d", format!("{}/test-0/src/pages", made[0])),
        ("e", format!("{}/nope", made[0])),
        ("r", made[0].clone()),
        ("d", made[1].clone()),
        ("d", made[2].clone()),
    ] {
        assert!(keys.contains(&format!("{kind}:{p}")), "{keys:?}");
    }
    assert_eq!(v["watches"], serde_json::json!([made[0]]));
    let fixed = &made[3];
    let kept: Vec<&String> = keys.iter().filter(|k| k.contains("quod-fixed-")).collect();
    assert_eq!(
        kept,
        vec![
            &format!("d:{fixed}"),
            &format!("e:{fixed}"),
            &format!("e:{fixed}/after"),
            &format!("sl:{fixed}"),
        ],
        "creation and removal must not suppress any observation"
    );
    let b = base.display().to_string();
    assert!(keys.contains(&format!("f:{b}/pre/x.txt")), "{keys:?}");
    assert!(keys.contains(&format!("d:{b}/ws/src/made")), "{keys:?}");
}

#[test]
fn fresh_directory_observations_keep_native_dependencies() {
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("no node: skipped");
        return;
    }
    let regression =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/vitest/owned-directory-record.test.mjs");
    let out = Command::new("node")
        .arg("--test")
        .arg(regression)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn listing_and_copying_the_environment_conservatively_key_values() {
    // Object.keys asks for descriptors but discards their values. The same
    // trap can return a value directly to application code, so without a
    // consumer proof it must retain env:, including per-run CI variables.
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
patchEnv(r)
Object.keys(process.env).filter((k) => /^debug_/i.test(k))
const listed = [...r.keys].filter((k) => k.endsWith(':GITHUB_SHA'))
const copy = {{ ...process.env }}
const copied = [...r.keys].filter((k) => k.endsWith(':GITHUB_SHA'))
console.log(JSON.stringify({{ listed, copied, all: r.keys.has('envall:') }}))
"#,
        record = record.display().to_string(),
    );
    let s = t.path().join("t.mjs");
    fs::write(&s, script).unwrap();
    let out = Command::new("node")
        .arg(&s)
        .env("GITHUB_SHA", "abc")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["listed"], serde_json::json!(["env:GITHUB_SHA"]));
    assert_eq!(v["copied"], serde_json::json!(["env:GITHUB_SHA"]));
    assert_eq!(v["all"], true);
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
         export const names = Object.keys(pkg.dependencies)\n\
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
        serde_json::json!([
            "pjf:package.json\u{0}[[\"dependencies\",\"zod-4.1\"],[\"dependencies\"]]"
        ]),
        "{v}"
    );
    assert_eq!(v["variable"], serde_json::json!(["f:package.json"]), "{v}");
    assert_eq!(v["names"], serde_json::json!(["zod-4.1"]), "{v}");
}

#[test]
fn access_queries_keep_permissions_separate_from_package_identity() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    write(&root.join("node_modules/tool/bin"), "not executable");
    let Some(v) = main_recorder(
        root,
        r#"
const fs = req('node:fs')
try { fs.accessSync('node_modules/tool/bin', fs.constants.X_OK) } catch {}
console.log(JSON.stringify([...r.keys].filter(k => k.startsWith('a1:')).map(k => k.replace(process.cwd() + '/', ''))))
"#,
    ) else {
        return;
    };
    assert_eq!(v, serde_json::json!(["a1:node_modules/tool/bin"]));
}

#[test]
fn dependency_values_are_inputs_even_when_the_package_is_not_loaded() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    write(
        &root.join("package.json"),
        r#"{"devDependencies":{"unloaded":"1"}}"#,
    );
    write(
        &root.join("cfg/names.mjs"),
        "import {readFileSync} from 'node:fs'\nconst pkg = JSON.parse(readFileSync('package.json', 'utf8'))\nexport const value = Object.keys(pkg.devDependencies)\n",
    );
    write(
        &root.join("cfg/value.mjs"),
        "import {readFileSync} from 'node:fs'\nconst pkg = JSON.parse(readFileSync('package.json', 'utf8'))\nexport const value = pkg.devDependencies.unloaded === '1' ? 'suite-a' : 'suite-b'\n",
    );
    let Some(v) = main_recorder(
        root,
        r#"
const fields = () => [...settledKeys(r)].filter(k => k.startsWith('pjf:')).map(k => JSON.parse(k.split('\0')[1]))
await import('./cfg/names.mjs')
const names = fields()
r.keys.clear(); r.pjReads.clear(); r.pjParsed.clear(); r.pjUse.clear()
const m = await import('./cfg/value.mjs')
console.log(JSON.stringify({names, values: fields(), choice: m.value}))
"#,
    ) else {
        return;
    };
    assert_eq!(v["names"], serde_json::json!([[["devDependencies"]]]));
    assert_eq!(
        v["values"],
        serde_json::json!([[["devDependencies", "unloaded"], ["devDependencies"]]])
    );
    assert_eq!(v["choice"], "suite-a");
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

/// Runs `main` (a script path) under child.mjs, the preload of a node child
/// a recorded test file starts, with GitHub's variables of a push; returns
/// the child's dump.
fn child_dump(root: &Path, main: &Path) -> Option<serde_json::Value> {
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("no node: skipped");
        return None;
    }
    let child = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/vitest/child.mjs");
    let out = root.join("dump.json");
    let st = Command::new("node")
        .arg("--import")
        .arg(&child)
        .arg(main)
        .current_dir(root)
        .env("QUOD_VITEST_CHILD", &out)
        .env("QUOD_VITEST_STATE", root)
        .env("GITHUB_ACTIONS", "true")
        .env("GITHUB_EVENT_NAME", "push")
        .env("GITHUB_SHA", "abc")
        .output()
        .unwrap();
    assert!(
        st.status.success(),
        "{}",
        String::from_utf8_lossy(&st.stderr)
    );
    Some(serde_json::from_slice(&fs::read(&out).unwrap()).unwrap())
}

// A stand-in for ci-info: its module-init reads of GitHub's variables
// (isPR is GITHUB_EVENT_NAME's value).
const CI_INFO: &str = "const env = process.env;\n\
exports.isCI = !!env.GITHUB_ACTIONS;\n\
exports.isPR = env.GITHUB_EVENT_NAME === 'pull_request';\n";

#[test]
fn npms_allowance_is_by_process_and_presence_keyed() {
    // In npm or npx itself (argv[1] is npm-cli.js or npx-cli.js), npm's
    // reads of the CI's per-run variables, its bundled ci-info's included,
    // are keyed by presence (envp:), never by value and never not at all.
    // The same modules loaded by any other program are keyed by value: the
    // allowance is the process's, not a stack frame's.
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    let npm = root.join("lib/node_modules/npm");
    write(&npm.join("node_modules/ci-info/index.js"), CI_INFO);
    let cli = "const ci = require('../node_modules/ci-info');\n\
               const copy = { ...process.env };\n\
               if (process.env.GITHUB_SHA === undefined) process.exit(3);\n\
               module.exports = ci;\n";
    write(&npm.join("bin/npx-cli.js"), cli);
    write(&npm.join("bin/other.js"), cli);
    let Some(v) = child_dump(root, &npm.join("bin/npx-cli.js")) else {
        return;
    };
    let keys = |v: &serde_json::Value| -> Vec<String> {
        v["keys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| k.as_str().unwrap().to_string())
            .filter(|k| k.starts_with("env"))
            .collect()
    };
    let k = keys(&v);
    for n in ["GITHUB_ACTIONS", "GITHUB_EVENT_NAME", "GITHUB_SHA"] {
        assert!(
            k.contains(&format!("envp:{n}")),
            "{n} not presence-keyed: {k:?}"
        );
        assert!(
            !k.contains(&format!("env:{n}")),
            "{n} keyed by value in npx: {k:?}"
        );
    }
    assert!(k.contains(&"envtool:".to_string()), "{k:?}");
    assert!(!k.contains(&"envall:".to_string()), "{k:?}");

    // Not npm: the very same frames under /node_modules/npm/ key values.
    let Some(v) = child_dump(root, &npm.join("bin/other.js")) else {
        return;
    };
    let k = keys(&v);
    for n in ["GITHUB_ACTIONS", "GITHUB_EVENT_NAME", "GITHUB_SHA"] {
        assert!(
            k.contains(&format!("env:{n}")),
            "{n} not keyed outside npm: {k:?}"
        );
    }
    assert!(k.contains(&"envall:".to_string()), "{k:?}");
}

#[test]
fn ci_info_in_a_test_files_worker_keys_the_event_by_value() {
    // A test file importing ci-info evaluates it with only ci-info's frames
    // on the stack. Its isPR is GITHUB_EVENT_NAME's value: keyed by value,
    // or a record made on a push replays on a pull request.
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("no node: skipped");
        return;
    }
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    write(&root.join("node_modules/ci-info/index.js"), CI_INFO);
    // Loaded the way a test runner loads an external module: nothing but
    // ci-info's frames (and node's) on the stack, no await chain back to
    // the importer.
    write(
        &root.join("node_modules/vitest/dist/load.mjs"),
        "export default (p, done) => setImmediate(() => import(p).then((m) => setImmediate(() => done(m))))\n",
    );
    let record = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/vitest/record.mjs");
    let script = format!(
        r#"
import {{ createRecorder, patchEnv }} from {record:?}
const r = createRecorder({{ stateDir: '/nonexistent-state', mode: 'worker' }})
patchEnv(r)
const {{ default: load }} = await import('./node_modules/vitest/dist/load.mjs')
const ci = await new Promise((done) => load(new URL('./node_modules/ci-info/index.js', import.meta.url).href, done))
console.log(JSON.stringify({{ keys: [...r.keys], isPR: ci.default.isPR }}))
"#,
        record = record.display().to_string(),
    );
    let s = root.join("t.mjs");
    fs::write(&s, script).unwrap();
    let out = Command::new("node")
        .arg(&s)
        .current_dir(root)
        .env("GITHUB_ACTIONS", "true")
        .env("GITHUB_EVENT_NAME", "push")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let has = |k: &str| v["keys"].as_array().unwrap().iter().any(|x| x == k);
    assert_eq!(v["isPR"], false, "{v}");
    assert!(has("env:GITHUB_EVENT_NAME"), "{v}");
    assert!(has("env:GITHUB_ACTIONS"), "{v}");
}

#[test]
fn caller_attribution_cache_keeps_user_frames_and_resets_across_async_boundaries() {
    if Command::new("node").arg("--version").output().is_err() {
        return;
    }
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    write(
        &root.join("node_modules/plugin/index.cjs"),
        "module.exports = fn => fn('PLUGIN_');",
    );
    let record = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/vitest/record.mjs");
    let script = format!(
        r#"
const {{ getCallSites }} = require('node:util');
(async () => {{
  const {{ createRecorder, patchEnv }} = await import({record:?});
  const r = createRecorder({{ stateDir: '/no-state', mode: 'main' }});
  patchEnv(r);
  let formatted = 0;
  const formatter = () => (e, frames) => {{ formatted++; return 'Error\n' + frames.map(f => '    at ' + f.toString()).join('\n') }};
  Error.prepareStackTrace = formatter();
  function read(prefix) {{ for (let i=0; i<20; i++) void process.env[prefix+i]; }}
  const counts=[];
  for (let i=0; i<3; i++) {{
    if (i===1) await Promise.resolve();
    if (i===2) Error.prepareStackTrace=formatter();
    const before=formatted; read('QUOD_CHECK_'+i+'_'); counts.push(formatted-before);
  }}
  require('../plugin/index.cjs')(read);
  function deep(n) {{ if(n) deep(n-1); else read('QUOD_DEEP_'); }}
  const before=formatted; deep(230); const deepCount=formatted-before;
  console.log(JSON.stringify({{ native: typeof getCallSites==='function', counts, deepCount, user: r.keys.has('env:PLUGIN_0') }}));
}})();
"#,
        record = record.display().to_string()
    );
    let s = root.join("node_modules/vite/reader.cjs");
    write(&s, &script);
    let out = Command::new("node").arg(s).output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["user"], true, "a plugin's reads must remain inputs: {v}");
    if v["native"] == true {
        for n in v["counts"].as_array().unwrap() {
            assert!(n.as_u64().unwrap() > 0 && n.as_u64().unwrap() < 5, "{v}");
        }
    }
    assert!(
        v["deepCount"].as_u64().unwrap() >= 20,
        "truncated native stacks must use the full path: {v}"
    );
}
