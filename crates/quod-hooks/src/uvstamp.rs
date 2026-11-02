//! uv's editable-install cache stamps, pinned after an install step.
//!
//! `uv sync` installs a local project (the repository itself, usually
//! editable) and writes `<name>.dist-info/uv_cache.json` beside it: the
//! wall-clock mtimes of the project's cache keys (`pyproject.toml`, the
//! `src` directory, ...) when it built, which uv compares on its next sync
//! to decide whether to rebuild. The dist-info's `RECORD` lists that file
//! with its SHA-256. A fresh checkout has fresh mtimes, so every install of
//! the same commit wrote different bytes into both files; the venv is a
//! sealed prefix recorded whole (quod-cli's seal.rs), and every step that
//! ran in it read new inputs and missed (python-attrs/attrs on
//! bench-real-small: 0 of 65 test steps replayed).
//!
//! `pin_uv_cache_stamps` sets every timestamp in such a file to zero (the
//! epoch) and rewrites the file's `RECORD` line to the new hash and size,
//! nothing else. What a later step sees changes only in uv's private
//! freshness stamp: Python never reads it, `importlib.metadata` reads
//! `RECORD` (which stays a correct record of the files beside it), and a
//! later `uv sync` or `uv run` compares the stamp with the project's
//! current mtimes, which zero never equals, so uv rebuilds the project as
//! it does on any fresh checkout (and as it already did under Quod, where
//! the observed tree serves content-derived mtimes that never matched the
//! install's wall clock either). A stamp is only pinned when the file is
//! the JSON object uv writes and its `RECORD` line holds its current hash
//! and size; anything else is left as the step wrote it.

use std::path::{Path, PathBuf};

use anyhow::Context;

/// The file uv writes in an installed local project's dist-info.
pub const UV_CACHE_JSON: &str = "uv_cache.json";

/// Pins, under `root` (a step's upper dir, or the venv a network step
/// wrote), every `site-packages/*.dist-info/uv_cache.json` as the module
/// doc says; returns how many files were rewritten.
pub fn pin_uv_cache_stamps(root: &Path) -> anyhow::Result<usize> {
    let mut sites = Vec::new();
    find_site_packages(root, &mut sites, 0);
    let mut n = 0;
    for site in sites {
        let Ok(rd) = std::fs::read_dir(&site) else {
            continue;
        };
        for item in rd.flatten() {
            let name = item.file_name().to_string_lossy().into_owned();
            if name.ends_with(".dist-info")
                && item.file_type().is_ok_and(|t| t.is_dir())
                && pin_one(&item.path(), &name)?
            {
                n += 1;
            }
        }
    }
    Ok(n)
}

/// Every directory named `site-packages` under `dir` (not under another
/// one, and not through symlinks: a venv's `lib64` is a link to `lib`).
fn find_site_packages(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    // A venv's site-packages is 3 levels down (lib/python3.12/...); a
    // workspace may hold venvs a few levels deep. Bounded so a huge upper
    // dir (node_modules) costs little.
    if depth > 8 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for item in rd.flatten() {
        if !item.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let name = item.file_name();
        if name == "site-packages" {
            out.push(item.path());
        } else if name != "node_modules" && name != ".git" {
            find_site_packages(&item.path(), out, depth + 1);
        }
    }
}

/// `uv_cache.json` with every `"secs_since_epoch":N` and
/// `"nanos_since_epoch":N` set to 0; `None` when the text is not a JSON
/// object with a `timestamp` (not uv's) or nothing changes.
pub fn zero_stamps(json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    if !v.as_object().is_some_and(|o| o.contains_key("timestamp")) {
        return None;
    }
    let mut out = String::with_capacity(json.len());
    let mut rest = json;
    loop {
        let next = ["\"secs_since_epoch\":", "\"nanos_since_epoch\":"]
            .iter()
            .filter_map(|k| rest.find(k).map(|i| (i, k.len())))
            .min();
        let Some((i, klen)) = next else { break };
        let from = i + klen;
        let digits = rest[from..].bytes().take_while(u8::is_ascii_digit).count();
        out.push_str(&rest[..from]);
        if digits > 0 {
            out.push('0');
        }
        rest = &rest[from + digits..];
    }
    out.push_str(rest);
    // Still the same JSON shape, every stamp zero.
    let w: serde_json::Value = serde_json::from_str(&out).ok()?;
    (w.as_object().map(|o| o.len()) == v.as_object().map(|o| o.len()) && out != json).then_some(out)
}

fn pin_one(dist_info: &Path, name: &str) -> anyhow::Result<bool> {
    let stamp = dist_info.join(UV_CACHE_JSON);
    let record = dist_info.join("RECORD");
    let (Ok(sm), Ok(rm)) = (
        std::fs::symlink_metadata(&stamp),
        std::fs::symlink_metadata(&record),
    ) else {
        return Ok(false);
    };
    if !sm.is_file() || !rm.is_file() {
        return Ok(false);
    }
    let old = std::fs::read(&stamp).with_context(|| format!("read {}", stamp.display()))?;
    let Some(new) = std::str::from_utf8(&old).ok().and_then(zero_stamps) else {
        return Ok(false);
    };
    let rec = std::fs::read(&record).with_context(|| format!("read {}", record.display()))?;
    let Ok(rec) = String::from_utf8(rec) else {
        return Ok(false);
    };
    let path = format!("{name}/{UV_CACHE_JSON}");
    let want = record_line(&path, &old);
    let mut found = 0;
    let lines: Vec<String> = rec
        .split_inclusive('\n')
        .map(|l| {
            let (body, nl) = match l.strip_suffix("\r\n") {
                Some(b) => (b, "\r\n"),
                None => match l.strip_suffix('\n') {
                    Some(b) => (b, "\n"),
                    None => (l, ""),
                },
            };
            if body == want {
                found += 1;
                format!("{}{nl}", record_line(&path, new.as_bytes()))
            } else {
                l.to_string()
            }
        })
        .collect();
    // The record must vouch for the file as it is, exactly once.
    if found != 1 {
        return Ok(false);
    }
    replace_file(&stamp, new.as_bytes(), &sm)?;
    replace_file(&record, lines.concat().as_bytes(), &rm)?;
    Ok(true)
}

/// A RECORD line (the wheel spec): path, `sha256=` and the urlsafe
/// unpadded base64 of the digest, size.
fn record_line(path: &str, content: &[u8]) -> String {
    format!(
        "{path},sha256={},{}",
        b64url_nopad(&sha256(content)),
        content.len()
    )
}

/// Writes `bytes` to a new file beside `path` with `path`'s mode, then
/// renames it over `path` (a hard link to a cache, under another link
/// mode, is never edited in place).
fn replace_file(path: &Path, bytes: &[u8], m: &std::fs::Metadata) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let tmp = path.with_extension("quod-tmp");
    std::fs::write(&tmp, bytes).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::set_permissions(
        &tmp,
        std::fs::Permissions::from_mode(m.permissions().mode()),
    )?;
    std::fs::rename(&tmp, path).with_context(|| format!("rename onto {}", path.display()))?;
    Ok(())
}

fn b64url_nopad(b: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(b.len().div_ceil(3) * 4);
    for c in b.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..=c.len() {
            out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

/// SHA-256 (FIPS 180-4), for RECORD lines.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in msg.chunks(64) {
        let mut w = [0u32; 64];
        for (i, c) in block.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 32];
    for (i, x) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&x.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn sha256_and_record_encoding_match_the_wheel_tools() {
        assert_eq!(
            hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let long = vec![b'a'; 1_000];
        assert_eq!(
            hex(&sha256(&long)),
            "41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3"
        );
        // Lines uv wrote (attrs' editable install on bench-real-small).
        assert_eq!(
            record_line("attrs-0.1.dev1.dist-info/INSTALLER", b"uv"),
            "attrs-0.1.dev1.dist-info/INSTALLER,sha256=5hhM4Q4mYTT9z6QB6PGpUAW81PGNFrYrdXMj4oM_6ak,2"
        );
        assert_eq!(
            record_line("attrs-0.1.dev1.dist-info/REQUESTED", b""),
            "attrs-0.1.dev1.dist-info/REQUESTED,sha256=47DEQpj8HBSa-_TImW-5JCeuQeRkm5NMpJWZG3hSuFU,0"
        );
        assert_eq!(
            record_line("attrs-0.1.dev1.dist-info/uv_build.json", b"{}"),
            "attrs-0.1.dev1.dist-info/uv_build.json,sha256=RBNvo1WzZ4oRRq0W9-hknpT7T8If536DEMBg9hyq_4o,2"
        );
    }

    const STAMP_A: &str = r#"{"timestamp":{"secs_since_epoch":1793645755,"nanos_since_epoch":259166410},"commit":null,"tags":null,"env":{},"directories":{"src":{"secs_since_epoch":1793645755,"nanos_since_epoch":259166410}}}"#;
    const STAMP_B: &str = r#"{"timestamp":{"secs_since_epoch":1793645779,"nanos_since_epoch":750411263},"commit":null,"tags":null,"env":{},"directories":{"src":{"secs_since_epoch":1793645779,"nanos_since_epoch":750411263}}}"#;

    #[test]
    fn stamps_are_zeroed_and_nothing_else() {
        let z = zero_stamps(STAMP_A).unwrap();
        assert_eq!(
            z,
            r#"{"timestamp":{"secs_since_epoch":0,"nanos_since_epoch":0},"commit":null,"tags":null,"env":{},"directories":{"src":{"secs_since_epoch":0,"nanos_since_epoch":0}}}"#
        );
        assert_eq!(zero_stamps(STAMP_B).unwrap(), z);
        // A commit and tags are kept: they are the same for the same commit.
        let c = r#"{"timestamp":{"secs_since_epoch":5,"nanos_since_epoch":6},"commit":"9d9f94a0","tags":"abc","env":{"X":"1"},"directories":{}}"#;
        assert_eq!(
            zero_stamps(c).unwrap(),
            r#"{"timestamp":{"secs_since_epoch":0,"nanos_since_epoch":0},"commit":"9d9f94a0","tags":"abc","env":{"X":"1"},"directories":{}}"#
        );
        // Already pinned, not uv's shape, not JSON: nothing.
        assert_eq!(zero_stamps(&z), None);
        assert_eq!(zero_stamps(r#"{"secs_since_epoch":5}"#), None);
        assert_eq!(zero_stamps("not json"), None);
    }

    fn install(root: &Path, stamp: &str, record_hash_of: &str) -> PathBuf {
        let di = root.join("venv/lib/python3.12/site-packages/attrs-26.1.dist-info");
        std::fs::create_dir_all(&di).unwrap();
        std::fs::write(di.join("METADATA"), "Name: attrs\n").unwrap();
        std::fs::write(di.join(UV_CACHE_JSON), stamp).unwrap();
        let rec = format!(
            "{}\nattrs-26.1.dist-info/RECORD,,\n{}\n",
            record_line("attrs-26.1.dist-info/METADATA", b"Name: attrs\n"),
            record_line(
                "attrs-26.1.dist-info/uv_cache.json",
                record_hash_of.as_bytes()
            ),
        );
        std::fs::write(di.join("RECORD"), rec).unwrap();
        di
    }

    #[test]
    fn two_installs_of_one_commit_become_the_same_bytes() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let da = install(a.path(), STAMP_A, STAMP_A);
        let db = install(b.path(), STAMP_B, STAMP_B);
        // lib64 -> lib, as uv makes it: walked once.
        std::os::unix::fs::symlink("lib", a.path().join("venv/lib64")).unwrap();
        assert_eq!(pin_uv_cache_stamps(a.path()).unwrap(), 1);
        assert_eq!(pin_uv_cache_stamps(b.path()).unwrap(), 1);
        for f in [UV_CACHE_JSON, "RECORD", "METADATA"] {
            assert_eq!(
                std::fs::read(da.join(f)).unwrap(),
                std::fs::read(db.join(f)).unwrap(),
                "{f}"
            );
        }
        // The RECORD stays true to the file, and the other lines stay.
        let rec = std::fs::read_to_string(da.join("RECORD")).unwrap();
        let stamp = std::fs::read(da.join(UV_CACHE_JSON)).unwrap();
        assert!(rec.contains(&record_line("attrs-26.1.dist-info/uv_cache.json", &stamp)));
        assert!(rec.contains(&record_line(
            "attrs-26.1.dist-info/METADATA",
            b"Name: attrs\n"
        )));
        assert!(rec.contains("attrs-26.1.dist-info/RECORD,,\n"));
        // Idempotent.
        assert_eq!(pin_uv_cache_stamps(a.path()).unwrap(), 0);
    }

    #[test]
    fn a_record_that_does_not_vouch_for_the_stamp_leaves_both_alone() {
        let t = tempfile::tempdir().unwrap();
        let di = install(t.path(), STAMP_A, STAMP_B);
        let rec = std::fs::read(di.join("RECORD")).unwrap();
        assert_eq!(pin_uv_cache_stamps(t.path()).unwrap(), 0);
        assert_eq!(
            std::fs::read(di.join(UV_CACHE_JSON)).unwrap(),
            STAMP_A.as_bytes()
        );
        assert_eq!(std::fs::read(di.join("RECORD")).unwrap(), rec);
    }
}
