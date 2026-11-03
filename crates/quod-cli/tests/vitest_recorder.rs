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
    write(&root.join("package.json"), r#"{"name":"x","version":"1.0.0"}"#);
    let reader = "const fs = require('node:fs');\nmodule.exports = (p) => JSON.parse(fs.readFileSync(p, 'utf8'));\n";
    write(&root.join("node_modules/vite/dist/resolve.cjs"), reader);
    write(&root.join("node_modules/vite-plugin-version/index.cjs"), reader);
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
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let pj = root.join("package.json").display().to_string();
    assert_eq!(v["resolver"], serde_json::json!([format!("pj:{pj}")]));
    assert_eq!(v["plugin"], serde_json::json!([format!("f:{pj}")]));
}
