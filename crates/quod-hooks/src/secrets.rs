//! Secrets a step is given: salted in its key, masked in everything stored.
//!
//! The hook is never told which values are secrets: the container-hooks
//! protocol carries no mask list (confirmed in the lab), and the runner
//! masks its own list only in the logs it uploads. A value is classified a
//! secret (`classify`) by any of:
//!
//! - its name says so (`step::secret_like`), or its value has a secret's
//!   shape (`step::looks_secret`);
//! - its provenance: the workflow maps a secret to that name
//!   (`provenance`, read from the workflow file at the first step);
//! - it holds a value already known to be one of the job's secrets (an
//!   earlier step's, a container env secret, a `::add-mask::` value).
//!
//! GitHub-issued tokens are withheld from keyed steps altogether
//! (`step::withheld`). A classified value goes into the key as a salted
//! hash (per state directory, so a stored key hash cannot be checked
//! against guessed values elsewhere), every log blob is masked in each of
//! the encodings actions/runner masks (`masks_for`), a step whose outputs
//! hold one is not recorded, and a step given any stores no workspace
//! outputs at all. A classified value shorter than `MIN_SECRET_LEN` cannot
//! be masked reliably: the step is not recorded.
//!
//! The values themselves stay out of `<state>`'s store, ledgers and events:
//! the job's known secrets (`JobSecrets`) live in RAM-backed `/dev/shm` for
//! the job's life (or, where that cannot be used, in the job's directory,
//! mode 0600, gone at cleanup).

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};

use aho_corasick::AhoCorasick;
use anyhow::{Context, bail};
use quod_cas::Hash;
use serde::{Deserialize, Serialize};

use crate::provenance::Provenance;
use crate::step;

/// A classified value shorter than this is too short to mask reliably (any
/// occurrence of its few bytes would be masked, and its encodings are
/// shorter still): the step is not recorded. GitHub masks values of any
/// length; Quod would rather not store.
pub const MIN_SECRET_LEN: usize = 8;

/// Fragments shorter than this (a line of a multi-line secret, an encoding
/// of a short one) are not masked on their own: they would mask ordinary
/// text. A whole value always is, when it is at least this long.
const MIN_FRAGMENT: usize = 4;

/// The state's secret salt, created on first use (32 random bytes, 0600).
pub fn salt(state_dir: &Path) -> anyhow::Result<[u8; 32]> {
    let p = state_dir.join("salt");
    if let Ok(b) = std::fs::read(&p)
        && let Ok(s) = <[u8; 32]>::try_from(b.as_slice())
    {
        return Ok(s);
    }
    let mut s = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut s))
        .context("read /dev/urandom")?;
    std::fs::create_dir_all(state_dir)?;
    let tmp = state_dir.join(format!("salt.tmp.{}", ulid::Ulid::generate()));
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(&s)?;
        f.sync_all()?;
    }
    // Two first uses at once: the first rename wins, both read it back.
    if std::fs::hard_link(&tmp, &p).is_err() {
        let _ = std::fs::remove_file(&tmp);
        let b = std::fs::read(&p)?;
        return <[u8; 32]>::try_from(b.as_slice())
            .map_err(|_| anyhow::anyhow!("corrupt {}", p.display()));
    }
    let _ = std::fs::remove_file(&tmp);
    Ok(s)
}

/// What the key holds instead of a secret value.
pub fn salted(salt: &[u8; 32], name: &str, value: &str) -> String {
    let mut h = blake3::Hasher::new_keyed(salt);
    h.update(name.as_bytes());
    h.update(&[0]);
    h.update(value.as_bytes());
    format!("quod:secret:{}", Hash::from_blake3(h.finalize()))
}

/// The byte strings to mask for one secret value: the value and each of
/// its lines (the runner masks multi-line secrets line by line), each in
/// the encodings actions/runner's masker adds for every secret
/// (`src/Sdk/DTLogging/Logging/ValueEncoders.cs`, registered in
/// `HostContext.cs`, read at actions/runner 110142c2): base64 of the value
/// and of the value without its first one or two bytes (a prefix such as
/// `user:` shifts where it falls in a base64 group), the command-line
/// escape (`"` as `\"`), the expression escape (`'` as `''`), the JSON
/// string escape, the URI data escape, the XML escape, the value inside
/// double quotes, and the parts around a PowerShell `&`. Quod adds: each
/// base64 form without padding and its aligned core (the part that does
/// not depend on the bytes around it inside a longer base64 stream, e.g.
/// a docker `auth`), base64url, lower-case percent escapes and `+` for a
/// space, JSON with non-ASCII as `\u` escapes and `/` as `\/`, the shell's
/// backslash and single-quote escapes, and hex in either case. Longest
/// first, deduplicated, fragments shorter than `MIN_FRAGMENT` left out.
pub fn masks_for(value: &str) -> Vec<Vec<u8>> {
    let mut bases: Vec<&str> = vec![value];
    bases.extend(value.lines().map(str::trim_end));
    let mut m: Vec<Vec<u8>> = Vec::new();
    for (i, b) in bases.into_iter().enumerate() {
        if b.len() < MIN_FRAGMENT {
            continue;
        }
        m.push(b.as_bytes().to_vec());
        // Encodings of the whole value, and of each line long enough to
        // carry meaning on its own.
        if i == 0 || b.len() >= MIN_SECRET_LEN {
            m.extend(encodings(b).into_iter().filter(|e| e.len() >= MIN_FRAGMENT));
        }
    }
    m.sort_by_key(|b| std::cmp::Reverse(b.len()));
    m.dedup();
    m
}

/// The encodings of one value (see `masks_for`), the value itself not
/// included.
fn encodings(v: &str) -> Vec<Vec<u8>> {
    let b = v.as_bytes();
    let mut out: Vec<String> = Vec::new();
    // Base64 at the three byte offsets, as the runner does (full, padded),
    // plus unpadded, the aligned core (whole 3-byte groups only) and
    // base64url of that core.
    for shift in 0..3 {
        let body = if b.len() > shift { &b[shift..] } else { b };
        let full = base64(body, STD);
        out.push(full.trim_end_matches('=').to_string());
        out.push(full);
        let core = &body[..body.len() / 3 * 3];
        if core.len() >= 6 {
            out.push(base64(core, STD));
            out.push(base64(core, URL));
        }
    }
    // The runner's escapes.
    out.push(v.replace('"', "\\\""));
    out.push(v.replace('\'', "''"));
    let json = serde_json::to_string(v).expect("a string serializes");
    let json = json[1..json.len() - 1].to_string();
    out.push(json.replace('/', "\\/"));
    out.push(json);
    let mut ascii = String::new();
    for c in v.chars() {
        match c {
            '"' => ascii.push_str("\\\""),
            '\\' => ascii.push_str("\\\\"),
            '\n' => ascii.push_str("\\n"),
            '\r' => ascii.push_str("\\r"),
            '\t' => ascii.push_str("\\t"),
            c if (c as u32) < 0x20 || (c as u32) > 0x7e => {
                let mut buf = [0u16; 2];
                for u in c.encode_utf16(&mut buf) {
                    ascii.push_str(&format!("\\u{u:04x}"));
                }
            }
            c => ascii.push(c),
        }
    }
    out.push(ascii);
    let pct = |upper: bool, plus: bool| -> String {
        b.iter()
            .map(|&c| {
                if c.is_ascii_alphanumeric() || b"-._~".contains(&c) {
                    (c as char).to_string()
                } else if plus && c == b' ' {
                    "+".to_string()
                } else if upper {
                    format!("%{c:02X}")
                } else {
                    format!("%{c:02x}")
                }
            })
            .collect()
    };
    out.push(pct(true, false));
    out.push(pct(false, false));
    out.push(pct(true, true));
    let mut xml = String::new();
    for c in v.chars() {
        match c {
            '&' => xml.push_str("&amp;"),
            '<' => xml.push_str("&lt;"),
            '>' => xml.push_str("&gt;"),
            '"' => xml.push_str("&quot;"),
            '\'' => xml.push_str("&apos;"),
            c => xml.push(c),
        }
    }
    out.push(xml);
    if v.len() > 2 && v.starts_with('"') && v.ends_with('"') {
        out.push(v[1..v.len() - 1].to_string());
    }
    // The parts around a PowerShell `&` (the runner's pre/post ampersand
    // escapes), at least 6 long.
    let mut amp = Vec::new();
    if let Some(i) = v.find("&+") {
        amp.push(v[..i + 2].to_string());
        if v.len() > i + 3 {
            amp.push(v[i + 3..].to_string());
        }
    } else if let Some(i) = v.rfind('&') {
        amp.push(v[..=i].to_string());
        amp.push(v[i + 1..].to_string());
    }
    out.extend(amp.into_iter().filter(|s| s.len() >= 6));
    // The shell's escapes: a backslash before each special character
    // (bash's printf %q, set -x), and single quotes closed and reopened.
    let mut q = String::new();
    for c in v.chars() {
        if !(c.is_ascii_alphanumeric() || "-_./:=@%+,".contains(c)) {
            q.push('\\');
        }
        q.push(c);
    }
    out.push(q);
    out.push(v.replace('\'', "'\\''"));
    // Hex, either case.
    let hex: String = b.iter().map(|c| format!("{c:02x}")).collect();
    out.push(hex.to_uppercase());
    out.push(hex);
    let mut out: Vec<Vec<u8>> = out
        .into_iter()
        .filter(|s| s != v && !s.is_empty())
        .map(String::into_bytes)
        .collect();
    out.sort();
    out.dedup();
    out
}

const STD: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Base64 with `alphabet`, padded.
fn base64(bytes: &[u8], alphabet: &[u8; 64]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= c.len() {
                out.push(alphabet[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// `bytes` with every occurrence of a mask replaced by `***`.
pub fn mask(bytes: &[u8], masks: &[Vec<u8>]) -> Vec<u8> {
    if masks.is_empty() {
        return bytes.to_vec();
    }
    let ac = AhoCorasick::builder()
        .match_kind(aho_corasick::MatchKind::LeftmostLongest)
        .build(masks)
        .expect("mask patterns");
    let mut out = Vec::with_capacity(bytes.len());
    ac.replace_all_with_bytes(bytes, &mut out, |_, _, dst| {
        dst.extend_from_slice(b"***");
        true
    });
    out
}

/// A reason, a path or a log line with its secrets masked: what may go
/// into the ledger, job.json, events.jsonl or the step summary.
pub fn mask_str(s: &str, masks: &[Vec<u8>]) -> String {
    if masks.is_empty() {
        return s.to_string();
    }
    String::from_utf8_lossy(&mask(s.as_bytes(), masks)).into_owned()
}

/// Does `bytes` hold a mask?
pub fn holds(bytes: &[u8], masks: &[Vec<u8>]) -> bool {
    !masks.is_empty()
        && AhoCorasick::new(masks)
            .expect("mask patterns")
            .find(bytes)
            .is_some()
}

/// The first entry under `dir` that carries a mask: in a file's content,
/// in any entry's name or its path from `dir` (a step can write a secret as
/// a file name, or one holding `/` as a path), or in a symlink's target.
/// The path returned is relative to `dir` and masked: it goes into reasons,
/// never the secret.
pub fn file_containing(dir: &Path, masks: &[Vec<u8>]) -> anyhow::Result<Option<PathBuf>> {
    use std::os::unix::ffi::OsStrExt;
    if masks.is_empty() || !dir.exists() {
        return Ok(None);
    }
    let ac = AhoCorasick::new(masks).context("mask patterns")?;
    let report = |p: &Path| {
        let rel = p.strip_prefix(dir).unwrap_or(p);
        PathBuf::from(mask_str(&rel.to_string_lossy(), masks))
    };
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).with_context(|| format!("read {}", d.display()))? {
            let e = e?;
            let path = e.path();
            let rel = path.strip_prefix(dir).unwrap_or(&path);
            if ac.find(rel.as_os_str().as_bytes()).is_some() {
                return Ok(Some(report(&path)));
            }
            let t = e.file_type()?;
            if t.is_dir() {
                stack.push(path);
            } else if t.is_symlink() {
                let target = std::fs::read_link(&path)?;
                if ac.find(target.as_os_str().as_bytes()).is_some() {
                    return Ok(Some(report(&path)));
                }
            } else if t.is_file() {
                let f = std::fs::File::open(&path)?;
                if ac.stream_find_iter(f).next().is_some() {
                    return Ok(Some(report(&path)));
                }
            }
        }
    }
    Ok(None)
}

/// Which of a step's variables hold secrets, and what to mask.
#[derive(Clone, Debug, Default)]
pub struct Classified {
    /// The names of the classified variables, sorted.
    pub names: Vec<String>,
    /// The classified values, and the secret parts of templated ones.
    pub values: BTreeSet<String>,
    /// Everything to mask: the values and the job's known secrets, in every
    /// encoding.
    pub masks: Vec<Vec<u8>>,
    /// A classified variable whose value, or a secret part of it, is
    /// shorter than `MIN_SECRET_LEN`.
    pub short: Option<String>,
}

impl Classified {
    pub fn is_secret(&self, name: &str) -> bool {
        self.names
            .binary_search_by(|n| n.as_str().cmp(name))
            .is_ok()
    }
}

/// Is `(name, value)` no secret whatever its name: empty, or a placeholder
/// Quod itself put there.
fn exempt(name: &str, value: &str) -> bool {
    value.is_empty()
        || value == crate::surface::WITHHELD
        || step::placeholder(name).is_some_and(|p| p == value)
}

/// Classifies a step's environment (see the module docs): by name, by
/// shape, by provenance, and by holding a known secret of the job; then
/// any value holding a classified one. `known` is the job's secrets so far
/// (`JobSecrets::values`).
pub fn classify<'a>(
    env: impl IntoIterator<Item = (&'a String, &'a String)>,
    provenance: &Provenance,
    known: &BTreeSet<String>,
) -> Classified {
    let env: Vec<(&String, &String)> = env.into_iter().collect();
    let known_long: Vec<&String> = known.iter().filter(|k| k.len() >= MIN_SECRET_LEN).collect();
    let mut c = Classified::default();
    let mut names = BTreeSet::new();
    let mut values: BTreeSet<String> = BTreeSet::new();
    for (k, v) in &env {
        if exempt(k, v) {
            continue;
        }
        let by_name = step::secret_like(k) || provenance.names_secret(k);
        if by_name
            || step::looks_secret(v)
            || known.contains(v.as_str())
            || known_long.iter().any(|s| v.contains(s.as_str()))
        {
            names.insert((*k).clone());
            values.insert((*v).clone());
        }
    }
    // A value holding a classified one (a copy under another name, a URL
    // with a password in it) is one too.
    let long: Vec<String> = values
        .iter()
        .filter(|v| v.len() >= MIN_SECRET_LEN)
        .cloned()
        .collect();
    for (k, v) in &env {
        if !exempt(k, v) && !names.contains(k.as_str()) && long.iter().any(|s| v.contains(s)) {
            names.insert((*k).clone());
            values.insert((*v).clone());
        }
    }
    // The secret parts of a value the workflow builds around a secret
    // (`Bearer ${{ secrets.X }}`): a step can print the part alone.
    for (k, v) in &env {
        if names.contains(k.as_str()) {
            for part in provenance.secret_parts(k, v) {
                values.insert(part);
            }
        }
    }
    for v in &values {
        if v.len() < MIN_SECRET_LEN && c.short.is_none() {
            c.short = env
                .iter()
                .find(|(k, val)| {
                    names.contains(k.as_str()) && (*val == v || val.contains(v.as_str()))
                })
                .map(|(k, _)| (*k).clone());
        }
        c.masks.extend(masks_for(v));
    }
    for k in known {
        c.masks.extend(masks_for(k));
    }
    c.masks.sort_by_key(|b| std::cmp::Reverse(b.len()));
    c.masks.dedup();
    c.names = names.into_iter().collect();
    c.values = values;
    c
}

/// The values a step registered as secrets with `::add-mask::` (or the
/// old `##[add-mask]`) in its output, as the runner reads them: one per
/// line, the rest of the line after the command, trimmed.
pub fn add_masks(output: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    for line in output.split(|&b| b == b'\n') {
        let line = String::from_utf8_lossy(line);
        let line = line.trim();
        let rest = line
            .strip_prefix("::add-mask::")
            .or_else(|| line.strip_prefix("##[add-mask]"));
        if let Some(v) = rest {
            let v = v.trim();
            if !v.is_empty() {
                out.push(v.to_string());
            }
        }
    }
    out
}

/// What a job knows of its secrets, kept for its life outside `<state>`
/// (RAM-backed `/dev/shm`, mode 0600): the job container's env (job.json
/// holds it with classified values salted), and every value classified or
/// registered (`::add-mask::`) so far, so later steps mask them and treat a
/// copy of one under another name as a secret too.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct JobSecrets {
    pub container_env: BTreeMap<String, String>,
    pub values: BTreeSet<String>,
}

/// Where a job's `JobSecrets` live: `/dev/shm/quod-<state hash>/<job>`,
/// or, where /dev/shm cannot be written, the job directory (on disk; gone
/// at cleanup).
pub fn runtime_dir(state_dir: &Path) -> PathBuf {
    let abs = std::fs::canonicalize(state_dir).unwrap_or_else(|_| state_dir.to_path_buf());
    let tag = Hash::of(abs.to_string_lossy().as_bytes()).hex();
    PathBuf::from("/dev/shm").join(format!("quod-{}", &tag[..16]))
}

impl JobSecrets {
    /// The file, in `runtime_dir` when it is (or can be made) a directory
    /// of this user's alone: not a symlink, owned by the effective user,
    /// open to no one else. One that is not (another local user made it
    /// first, to read or plant a job's secrets) is refused. Where /dev/shm
    /// cannot be written: the job directory.
    fn path(state_dir: &Path, job_dir: &Path, id: &str) -> anyhow::Result<PathBuf> {
        let dir = runtime_dir(state_dir);
        let private = |dir: &Path| -> anyhow::Result<()> {
            use std::os::unix::fs::MetadataExt;
            let m = std::fs::symlink_metadata(dir)?;
            let me = std::fs::metadata("/proc/self").map(|m| m.uid()).ok();
            if !m.is_dir() || Some(m.uid()) != me || m.mode() & 0o077 != 0 {
                bail!(
                    "{} is not a private directory of this user (a symlink, another owner, or open to others): the job's secrets are not kept there",
                    dir.display()
                );
            }
            Ok(())
        };
        match std::fs::symlink_metadata(&dir) {
            Ok(_) => private(&dir)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if Self::make_dir(&dir).is_err() {
                    return Ok(job_dir.join("secrets.json"));
                }
                private(&dir)?;
            }
            Err(_) => return Ok(job_dir.join("secrets.json")),
        }
        Ok(dir.join(format!("{id}.json")))
    }

    fn make_dir(dir: &Path) -> std::io::Result<()> {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
    }

    pub fn load(state_dir: &Path, job_dir: &Path, id: &str) -> anyhow::Result<JobSecrets> {
        let p = Self::path(state_dir, job_dir, id)?;
        match std::fs::read(&p) {
            Ok(b) => {
                Ok(serde_json::from_slice(&b).with_context(|| format!("read {}", p.display()))?)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(JobSecrets::default()),
            Err(e) => Err(e).with_context(|| format!("read {}", p.display())),
        }
    }

    pub fn save(&self, state_dir: &Path, job_dir: &Path, id: &str) -> anyhow::Result<()> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let p = Self::path(state_dir, job_dir, id)?;
        let tmp = p.with_extension(format!("tmp.{}", ulid::Ulid::generate()));
        {
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&tmp)
                .with_context(|| format!("create {}", tmp.display()))?;
            f.write_all(&serde_json::to_vec(self)?)?;
        }
        std::fs::rename(&tmp, &p).with_context(|| format!("write {}", p.display()))?;
        Ok(())
    }

    pub fn remove(state_dir: &Path, job_dir: &Path, id: &str) {
        if let Ok(p) = Self::path(state_dir, job_dir, id) {
            let _ = std::fs::remove_file(p);
        }
        let _ = std::fs::remove_file(job_dir.join("secrets.json"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn salt_is_stable_and_salting_hides_the_value() {
        let tmp = tempfile::tempdir().unwrap();
        let a = salt(tmp.path()).unwrap();
        assert_eq!(salt(tmp.path()).unwrap(), a);
        let other = salt(&tmp.path().join("other")).unwrap();
        assert_ne!(a, other);
        let s = salted(&a, "NPM_TOKEN", "hunter2hunter2");
        assert!(s.starts_with("quod:secret:b3:") && !s.contains("hunter2"));
        assert_ne!(s, salted(&other, "NPM_TOKEN", "hunter2hunter2"));
        assert_ne!(s, salted(&a, "NPM_TOKEN", "hunter3hunter3"));
    }

    #[test]
    fn masking() {
        let m = masks_for("line-one-secret\nline-two-secret");
        let out = mask(
            b"a line-one-secret b line-two-secret\nline-one-secret\nline-two-secret",
            &m,
        );
        assert_eq!(out, b"a *** b ***\n***");
        assert_eq!(mask(b"abc", &masks_for("ab")), b"abc", "too short to mask");
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("a/b")).unwrap();
        std::fs::write(tmp.path().join("a/b/f"), "x line-two-secret y").unwrap();
        std::fs::write(tmp.path().join("a/g"), "nothing").unwrap();
        assert!(
            file_containing(tmp.path(), &m)
                .unwrap()
                .is_some_and(|p| p.ends_with("a/b/f"))
        );
        assert_eq!(
            file_containing(tmp.path(), &masks_for("absent-secret")).unwrap(),
            None
        );
        // A secret as a file name, or as a symlink's target.
        let named = tempfile::tempdir().unwrap();
        std::fs::write(named.path().join("x-line-one-secret.txt"), "nothing").unwrap();
        assert!(file_containing(named.path(), &m).unwrap().is_some());
        let linked = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("/tmp/line-two-secret", linked.path().join("l")).unwrap();
        assert!(file_containing(linked.path(), &m).unwrap().is_some());
    }

    /// Standard base64, padded, for the tests.
    fn b64(bytes: &[u8]) -> String {
        const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for c in bytes.chunks(3) {
            let n = (u32::from(c[0]) << 16)
                | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
                | u32::from(*c.get(2).unwrap_or(&0));
            for i in 0..4 {
                if i <= c.len() {
                    out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    fn masked(text: &str, secret: &str) -> String {
        String::from_utf8(mask(text.as_bytes(), &masks_for(secret))).unwrap()
    }

    /// GitHub's encodings of a secret (actions/runner ValueEncoders) are
    /// masked too: base64 at each of the three byte offsets a prefix can
    /// put it at, URL, JSON and shell escapes; and hex.
    #[test]
    fn encodings_are_masked() {
        // A docker config.json `auth`: base64("user:" + password). The
        // password starts at byte 5, offset 2 in a base64 group.
        let pw = "hunter2/hunter 2";
        for prefix in ["", "u:", "user:", "x"] {
            let auth = b64(format!("{prefix}{pw}").as_bytes());
            let out = masked(&format!("{{\"auth\":\"{auth}\"}}"), pw);
            assert!(
                out.contains("***") && !out.contains(&auth),
                "prefix {prefix:?}: {out}"
            );
        }
        assert!(!masked(&b64(pw.as_bytes()), pw).contains(&b64(pw.as_bytes())[..8]));
        // URL-encoded (p%40ss), JSON-escaped (pa\"ss), shell-escaped.
        let at = "p@ss-word-0123";
        assert_eq!(masked("u=p%40ss-word-0123&x", at), "u=***&x");
        assert_eq!(masked("u=p%40ss-word-0123", at), "u=***");
        let quote = "pa\"ss-word-0123";
        assert_eq!(
            masked(r#"{"p":"pa\"ss-word-0123"}"#, quote),
            r#"{"p":"***"}"#
        );
        assert_eq!(masked(r#"-e P=pa\"ss-word-0123"#, quote), "-e P=***");
        // Hex, either case.
        let hex: String = pw.bytes().map(|b| format!("{b:02x}")).collect();
        assert_eq!(masked(&format!("x {hex} y"), pw), "x *** y");
        assert_eq!(
            masked(&format!("x {} y", hex.to_uppercase()), pw),
            "x *** y"
        );
        // A PEM key with CRLF line ends: each line, the whole, and the
        // whole as JSON and as base64 (a Kubernetes secret's data).
        let pem = "-----BEGIN RSA PRIVATE KEY-----\r\nMIIEowIBAAKCAQEAuF1x2Qy7Vv1aZ9\r\nq8Lm3Nw0Tz5Ab7Cd9Ef1Gh3Ij5Kl7\r\n-----END RSA PRIVATE KEY-----\r\n";
        let json = serde_json::to_string(pem).unwrap();
        let out = masked(&json, pem);
        assert!(!out.contains("MIIEow") && !out.contains("q8Lm3N"), "{out}");
        let whole = b64(pem.as_bytes());
        let out = masked(&format!("data: {whole}"), pem);
        assert!(!out.contains(&whole[8..40]), "{out}");
        let out = masked("line q8Lm3Nw0Tz5Ab7Cd9Ef1Gh3Ij5Kl7 end", pem);
        assert_eq!(out, "line *** end");
    }

    fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn prov(names: &[&str]) -> Provenance {
        Provenance {
            names: names
                .iter()
                .map(|n| (n.to_string(), vec![vec![crate::provenance::Piece::Secret]]))
                .collect(),
            ..Provenance::default()
        }
    }

    /// A plain name the workflow maps a secret to is classified; so is a
    /// copy of a known secret under any name; placeholders never are; a
    /// short one is flagged.
    #[test]
    fn classification() {
        let e = env(&[
            ("APP_DSN", "s3cr3t-DSN value"),
            ("COPY", "postgres://u:s3cr3t-DSN value@db/x"),
            ("PLAIN", "hello world"),
            ("GOFLAGS", "-mod=mod"),
            (
                "ACTIONS_ID_TOKEN_REQUEST_URL",
                "http://192.0.2.1/quod-unreachable/",
            ),
            ("GITHUB_TOKEN", crate::surface::WITHHELD),
        ]);
        let none = BTreeSet::new();
        // Without provenance a plain name is not a secret (MAJOR 1 on main).
        assert!(classify(&e, &Provenance::default(), &none).names.is_empty());
        let c = classify(&e, &prov(&["APP_DSN"]), &none);
        assert_eq!(c.names, ["APP_DSN", "COPY"]);
        assert_eq!(c.short, None);
        assert_eq!(mask_str("x s3cr3t-DSN value y", &c.masks), "x *** y");
        // Known from an earlier step (or ::add-mask::): classified anywhere.
        let known: BTreeSet<String> = ["hello world".to_string()].into();
        let c = classify(&e, &Provenance::default(), &known);
        assert_eq!(c.names, ["PLAIN"]);
        // Too short to mask: flagged, whatever classified it.
        let c = classify(
            &env(&[("DB_PASSWORD", "short1x")]),
            &Provenance::default(),
            &none,
        );
        assert_eq!(c.short.as_deref(), Some("DB_PASSWORD"));
        let c = classify(&env(&[("X", "abc")]), &prov(&["X"]), &none);
        assert_eq!(c.short.as_deref(), Some("X"));
        // A templated value's secret part is masked on its own too.
        let p = Provenance {
            names: [(
                "AUTH".to_string(),
                vec![vec![
                    crate::provenance::Piece::Lit("Bearer ".into()),
                    crate::provenance::Piece::Secret,
                ]],
            )]
            .into(),
            ..Provenance::default()
        };
        let c = classify(&env(&[("AUTH", "Bearer tok-0123456789")]), &p, &none);
        assert_eq!(mask_str("got tok-0123456789", &c.masks), "got ***");
    }

    #[test]
    fn add_mask_commands() {
        assert_eq!(
            add_masks(b"a\n::add-mask::dyn-value-1\r\n  ::add-mask:: x2 \n##[add-mask]old\n::add-mask::\n"),
            ["dyn-value-1", "x2", "old"]
        );
        assert!(
            add_masks(b"echo ::add-mask::x").is_empty(),
            "only at a line's start"
        );
    }

    /// The job's secrets are kept outside the state and go at remove.
    #[test]
    fn job_secrets_live_outside_the_state() {
        let tmp = tempfile::tempdir().unwrap();
        let state = tmp.path().join("state");
        let job = state.join("jobs/J1");
        std::fs::create_dir_all(&job).unwrap();
        let s = JobSecrets {
            container_env: env(&[("K", "v-0123456789")]),
            values: ["v-0123456789".to_string()].into(),
        };
        s.save(&state, &job, "J1").unwrap();
        let back = JobSecrets::load(&state, &job, "J1").unwrap();
        assert_eq!(back.values, s.values);
        let mut files = Vec::new();
        let mut stack = vec![state.clone()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                if e.file_type().unwrap().is_dir() {
                    stack.push(e.path());
                } else {
                    files.push(std::fs::read(e.path()).unwrap());
                }
            }
        }
        let in_state = files
            .iter()
            .any(|f| f.windows(12).any(|w| w == b"v-0123456789"));
        assert_eq!(
            in_state,
            !runtime_dir(&state).join("J1.json").is_file(),
            "in /dev/shm, or (only where that cannot be used) the job directory"
        );
        // A runtime directory open to others is refused.
        let other = tmp.path().join("other-state");
        let dir = runtime_dir(&other);
        if std::fs::create_dir_all(&dir).is_ok() {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
            assert!(
                s.save(&other, &job, "J2").is_err(),
                "a shared directory is refused"
            );
            let _ = std::fs::remove_dir_all(&dir);
        }
        JobSecrets::remove(&state, &job, "J1");
        assert!(
            JobSecrets::load(&state, &job, "J1")
                .unwrap()
                .values
                .is_empty()
        );
    }

    /// The path `file_containing` reports never holds the secret itself: it
    /// goes into reasons, the job's state and the ledger. A secret as a file
    /// name, a directory name, a path spanning directories, a symlink target.
    #[test]
    fn a_secret_in_a_name_is_found_and_never_reported() {
        let secret = "s3cr3t-value-0123";
        let m = masks_for(secret);
        let check = |make: &dyn Fn(&Path)| {
            let tmp = tempfile::tempdir().unwrap();
            make(tmp.path());
            let p = file_containing(tmp.path(), &m)
                .unwrap()
                .expect("the secret is found");
            assert!(
                !p.to_string_lossy().contains(secret),
                "the reported path holds the secret: {}",
                p.display()
            );
        };
        check(&|d| std::fs::write(d.join(format!("x-{secret}.txt")), "").unwrap());
        check(&|d| std::fs::create_dir_all(d.join(format!("d-{secret}"))).unwrap());
        check(&|d| std::os::unix::fs::symlink(format!("/tmp/{secret}"), d.join("l")).unwrap());
        // A secret with a `/` spans directories: no one name holds it.
        let slash = "hunter2/hunter 2";
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(format!("f-{slash}"))).unwrap();
        let p = file_containing(tmp.path(), &masks_for(slash))
            .unwrap()
            .expect("a secret spanning directories is found");
        assert!(!p.to_string_lossy().contains(slash), "{}", p.display());
    }
}
