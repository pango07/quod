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
//! The values themselves stay out of `<state>`: the job's known secrets
//! (`JobSecrets`) live in RAM-backed `/dev/shm` for the job's life, and a
//! job does not run where that cannot be used. RAM-backed is not
//! swap-proof: tmpfs pages can be swapped out like any others.

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

/// `bytes` with every occurrence of a mask replaced by `***`, also where a
/// base64 stream is wrapped over lines (`base64` at 76 columns, openssl and
/// PEM at 64, any width): each run of lines of base64 characters alone is
/// matched as one string, so the secret's encoding is found whatever its
/// offset in the stream or its length; what it covers is masked on each line
/// it spans, the line breaks kept.
pub fn mask(bytes: &[u8], masks: &[Vec<u8>]) -> Vec<u8> {
    if masks.is_empty() {
        return bytes.to_vec();
    }
    let ac = AhoCorasick::builder()
        .match_kind(aho_corasick::MatchKind::LeftmostLongest)
        .build(masks)
        .expect("mask patterns");
    let covered = covered(&ac, bytes);
    if !covered.iter().any(|&c| c) {
        return bytes.to_vec();
    }
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if covered[i] {
            out.extend_from_slice(b"***");
            while i < bytes.len() && covered[i] {
                i += 1;
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

/// Which bytes of `bytes` a match covers: direct matches, and matches in
/// the joined view of each wrapped base64 run (its line breaks never
/// covered).
fn covered(ac: &AhoCorasick, bytes: &[u8]) -> Vec<bool> {
    let mut cov = vec![false; bytes.len()];
    for m in ac.find_iter(bytes) {
        cov[m.start()..m.end()].iter_mut().for_each(|c| *c = true);
    }
    for (joined, map) in base64_runs(bytes) {
        for m in ac.find_iter(&joined) {
            for &at in &map[m.start()..m.end()] {
                cov[at] = true;
            }
        }
    }
    cov
}

/// The runs of two or more consecutive lines of a base64 stream, each
/// joined without its line breaks, with the offset in `bytes` of every
/// joined byte. A line of the stream is base64 characters (standard or URL
/// alphabet, `=`) after its leading whitespace (an indented YAML block,
/// `sed 's/^/    /'`), a trailing CR allowed. A run may begin with the
/// base64 tail of a line that is not all base64 (`auth: <base64>`) and end
/// with the base64 head of the line after it.
fn base64_runs(bytes: &[u8]) -> Vec<(Vec<u8>, Vec<usize>)> {
    let b64 = |c: u8| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'/' | b'=' | b'-' | b'_');
    let mut runs = Vec::new();
    // (joined, offsets, pieces, whole lines): a run needs two pieces, one
    // of them a whole line, so two lines of ordinary text never make one.
    type Run = (Vec<u8>, Vec<usize>, usize, usize);
    let mut cur: Run = (Vec::new(), Vec::new(), 0, 0);
    let mut flush = |cur: &mut Run| {
        if cur.2 >= 2 && cur.3 >= 1 {
            runs.push((std::mem::take(&mut cur.0), std::mem::take(&mut cur.1)));
        }
        *cur = (Vec::new(), Vec::new(), 0, 0);
    };
    let push = |cur: &mut Run, from: usize, part: &[u8], whole: bool| {
        cur.0.extend_from_slice(part);
        cur.1.extend(from..from + part.len());
        cur.2 += 1;
        cur.3 += usize::from(whole);
    };
    let mut start = 0;
    for end in bytes
        .iter()
        .enumerate()
        .filter(|(_, b)| **b == b'\n')
        .map(|(i, _)| i)
        .chain(std::iter::once(bytes.len()))
    {
        let mut line = &bytes[start..end];
        if line.last() == Some(&b'\r') {
            line = &line[..line.len() - 1];
        }
        let lead = line.iter().take_while(|c| c.is_ascii_whitespace()).count();
        let (from, body) = (start + lead, &line[lead..]);
        if !body.is_empty() && body.iter().all(|&c| b64(c)) {
            push(&mut cur, from, body, true);
        } else {
            // The stream's last line may be the head of this one.
            let head = body.iter().take_while(|&&c| b64(c)).count();
            if cur.2 >= 1 && head > 0 {
                push(&mut cur, from, &body[..head], false);
            }
            flush(&mut cur);
            // And its first line the tail of this one.
            let tail = body.iter().rev().take_while(|&&c| b64(c)).count();
            if tail > 0 {
                push(
                    &mut cur,
                    from + body.len() - tail,
                    &body[body.len() - tail..],
                    false,
                );
            }
        }
        start = end + 1;
    }
    flush(&mut cur);
    runs
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
    if masks.is_empty() {
        return false;
    }
    let ac = AhoCorasick::new(masks).expect("mask patterns");
    ac.find(bytes).is_some()
        || base64_runs(bytes)
            .iter()
            .any(|(joined, _)| ac.find(joined).is_some())
}

/// Files up to this size are read whole by `file_containing`, so a wrapped
/// base64 stream in them is matched across its lines; larger ones are
/// streamed, direct matches only.
const WHOLE_FILE: u64 = 64 << 20;

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
                let found = if e.metadata()?.len() <= WHOLE_FILE {
                    holds(&std::fs::read(&path)?, masks)
                } else {
                    ac.stream_find_iter(std::fs::File::open(&path)?)
                        .next()
                        .is_some()
                };
                if found {
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
    /// Of those, the ones the job knows for certain are secrets: by the
    /// workflow's provenance, or related to a known secret (holding one,
    /// held by one, an `::add-mask::` value), at least `MIN_SECRET_LEN`
    /// long. Only these join the job's known secrets: a value flagged by
    /// its name or shape alone (GIT_AUTHOR_NAME holds `AUTH`) is salted and
    /// masked in its own step, not matched against every later one.
    pub strong: BTreeSet<String>,
    /// Everything to mask: the values and the job's known secrets, in every
    /// encoding.
    pub masks: Vec<Vec<u8>>,
    /// A classified variable whose value, or a secret part of it, is
    /// shorter than `MIN_SECRET_LEN`.
    pub short: Option<String>,
    /// That short value is a certain secret (see `strong`), not one flagged
    /// by name or shape alone.
    pub short_strong: bool,
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

/// `classify_with` for an environment no step has set anything in, in a
/// job whose image defines no variables.
pub fn classify<'a>(
    env: impl IntoIterator<Item = (&'a String, &'a String)>,
    provenance: &Provenance,
    known: &BTreeSet<String>,
) -> Classified {
    classify_with(env, provenance, known, &BTreeSet::new(), &BTreeMap::new())
}

/// Classifies a step's environment (see the module docs): by name, by
/// shape, by provenance, and by its relation to a known secret of the job
/// (equal to one, or holding one); then any value holding a classified one.
/// A value held by a known or classified secret (a password a step cut out
/// of a known DATABASE_URL) is one only under a name a step set: one in
/// `set_by_steps` (an earlier step's GITHUB_ENV), or one the workflow
/// assigns from a step's outputs (`Provenance::derived`). A name the
/// workflow or the runner sets otherwise is configuration, even when a
/// secret also holds its value (`production` in a dotenv secret,
/// `localhost` in a DSN). Such a held-by value is a secret of its step
/// only, never one of the job's (`strong`): nothing cascades from it.
/// `known` is the job's secrets so far (`JobSecrets::values`).
///
/// `image` is the job image's own config env (`Rootfs::env`). A variable
/// whose value is exactly the image's is defined by the image, not by a
/// workflow secret or the runner: the official python image's `GPG_KEY` is
/// the public fingerprint its release was verified with. Its name and shape
/// prove nothing about it (anyone who pulls the image reads it in the
/// config, and the image's root, which holds it, is in the state already),
/// so neither classifies it. Provenance and the job's known secrets still
/// do: a workflow that maps a secret onto the same name with the same value
/// is classified as before.
pub fn classify_with<'a>(
    env: impl IntoIterator<Item = (&'a String, &'a String)>,
    provenance: &Provenance,
    known: &BTreeSet<String>,
    set_by_steps: &BTreeSet<String>,
    image: &BTreeMap<String, String>,
) -> Classified {
    let env: Vec<(&String, &String)> = env.into_iter().collect();
    let from_image = |k: &str, v: &str| image.get(k).is_some_and(|i| i == v);
    let computed =
        |k: &str| set_by_steps.contains(k) || provenance.derived.contains(&k.to_ascii_uppercase());
    let long = |s: &&String| s.len() >= MIN_SECRET_LEN;
    let known_long: Vec<&String> = known.iter().filter(long).collect();
    let mut c = Classified::default();
    // name -> certain?
    let mut names: BTreeMap<String, bool> = BTreeMap::new();
    for (k, v) in &env {
        if exempt(k, v) {
            continue;
        }
        let strong = provenance.names_secret(k)
            || known.contains(v.as_str())
            || known_long.iter().any(|s| v.contains(s.as_str()));
        let held = v.len() >= MIN_SECRET_LEN
            && computed(k)
            && known_long.iter().any(|s| s.contains(v.as_str()));
        let heuristic = !from_image(k, v) && (step::secret_like(k) || step::looks_secret(v));
        if strong || held || heuristic {
            names.insert((*k).clone(), strong);
        }
    }
    // A value holding a classified one (a copy under another name, a URL
    // with a password in it) is one too, certain as the value it holds; one
    // held by a classified value, under a name a step set, is one of this
    // step's.
    let first: Vec<(String, bool)> = env
        .iter()
        .filter_map(|(k, v)| names.get(k.as_str()).map(|s| ((*v).clone(), *s)))
        .filter(|(v, _)| v.len() >= MIN_SECRET_LEN)
        .collect();
    for (k, v) in &env {
        if exempt(k, v) || names.contains_key(k.as_str()) || v.len() < MIN_SECRET_LEN {
            continue;
        }
        if let Some((_, s)) = first.iter().find(|(c, _)| v.contains(c.as_str())) {
            names.insert((*k).clone(), *s);
        } else if computed(k) && first.iter().any(|(c, _)| c.contains(v.as_str())) {
            names.insert((*k).clone(), false);
        }
    }
    let mut values: BTreeMap<String, bool> = BTreeMap::new();
    for (k, v) in &env {
        if let Some(&s) = names.get(k.as_str()) {
            let e = values.entry((*v).clone()).or_insert(s);
            *e |= s;
            // The secret parts of a value the workflow builds around a
            // secret (`Bearer ${{ secrets.X }}`): a step can print the part
            // alone.
            for part in provenance.secret_parts(k, v) {
                values.insert(part, true);
            }
        }
    }
    for (v, &strong) in &values {
        if v.len() < MIN_SECRET_LEN {
            if c.short.is_none() || (strong && !c.short_strong) {
                c.short = env
                    .iter()
                    .find(|(k, val)| names.contains_key(k.as_str()) && val.contains(v.as_str()))
                    .map(|(k, _)| (*k).clone());
                c.short_strong = strong;
            }
        } else if strong {
            c.strong.insert(v.clone());
        }
        c.masks.extend(masks_for(v));
    }
    for k in known {
        c.masks.extend(masks_for(k));
    }
    c.masks.sort_by_key(|b| std::cmp::Reverse(b.len()));
    c.masks.dedup();
    c.names = names.into_keys().collect();
    c.values = values.into_keys().collect();
    c
}

/// A workflow command line as actions/runner parses it
/// (`src/Runner.Worker/ActionCommandManager.cs`, `ActionCommand.TryParseV2`
/// and `TryParse` in `src/Sdk/DTPipelines/Pipelines/ActionCommand.cs`):
/// the trimmed line `::name[ k=v,...]::data` (or the old
/// `##[name k=v;...]data`), name compared without case, data unescaped
/// (`%0D` CR, `%0A` LF, `%25` %; and in the old form `%3B` ; and `%5D` ]).
/// `(name, data)`.
pub fn workflow_command(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    if let Some(rest) = line.strip_prefix("::") {
        let end = rest.find("::")?;
        let info = &rest[..end];
        let name = info.split(' ').next().unwrap_or("").to_ascii_lowercase();
        let data = rest[end + 2..]
            .replace("%0D", "\r")
            .replace("%0A", "\n")
            .replace("%25", "%");
        return Some((name, data));
    }
    if let Some(rest) = line.strip_prefix("##[") {
        let end = rest.find(']')?;
        let info = &rest[..end];
        let name = info.split(' ').next().unwrap_or("").to_ascii_lowercase();
        let data = rest[end + 1..]
            .replace("%3B", ";")
            .replace("%0D", "\r")
            .replace("%0A", "\n")
            .replace("%5D", "]")
            .replace("%25", "%");
        return Some((name, data));
    }
    None
}

/// The values a step registered as secrets with `::add-mask::` (any
/// properties form, `core.setSecret`'s escaping undone; or the old
/// `##[add-mask]`) in its output, as the runner reads them; each as given
/// and trimmed.
pub fn add_masks(output: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    for line in output.split(|&b| b == b'\n') {
        let line = String::from_utf8_lossy(line);
        if let Some((name, data)) = workflow_command(&line)
            && name == "add-mask"
        {
            for v in [data.as_str(), data.trim()] {
                if !v.is_empty() && !out.iter().any(|o: &String| o == v) {
                    out.push(v.to_string());
                }
            }
        }
    }
    out
}

/// Does a workflow command line (`::...` or `##[...`) in `output` hold a
/// mask? Replayed, the stored output would carry `***` where a fresh run
/// sets the value (`::set-output`, `::save-state`, ...): such a run is not
/// recordable.
pub fn command_line_holds(output: &[u8], masks: &[Vec<u8>]) -> bool {
    output.split(|&b| b == b'\n').any(|line| {
        let t = line.trim_ascii_start();
        (t.starts_with(b"::") || t.starts_with(b"##[")) && holds(line, masks)
    })
}

/// What a job knows of its secrets, kept for its life outside `<state>`
/// (RAM-backed `/dev/shm`, mode 0600; removed at cleanup and, for a job
/// that died, by the stale-job sweep): the job container's env (job.json
/// holds it with classified values salted), and every value classified or
/// registered (`::add-mask::`) so far, so later steps mask them and treat a
/// copy of one under another name as a secret too.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct JobSecrets {
    pub container_env: BTreeMap<String, String>,
    pub values: BTreeSet<String>,
}

/// Where a job's `JobSecrets` live: `/dev/shm/quod-<state hash>/<job>.json`.
pub fn runtime_dir(state_dir: &Path) -> PathBuf {
    let abs = std::fs::canonicalize(state_dir).unwrap_or_else(|_| state_dir.to_path_buf());
    let tag = Hash::of(abs.to_string_lossy().as_bytes()).hex();
    PathBuf::from("/dev/shm").join(format!("quod-{}", &tag[..16]))
}

impl JobSecrets {
    /// The file, in `runtime_dir`, which must be (or be made) a directory
    /// of this user's alone: not a symlink, owned by the effective user,
    /// open to no one else. One that is not (another local user made it
    /// first, to read or plant a job's secrets), or none at all (no
    /// writable /dev/shm), is refused: the values never go to disk.
    fn path(state_dir: &Path, _job_dir: &Path, id: &str) -> anyhow::Result<PathBuf> {
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
                Self::make_dir(&dir).with_context(|| {
                    format!(
                        "create {} (the job's secrets are kept in /dev/shm, never on disk)",
                        dir.display()
                    )
                })?;
                private(&dir)?;
            }
            Err(e) => {
                return Err(e).with_context(|| format!("stat {}", dir.display()));
            }
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
            // The data as given and trimmed (the runner masks what it is given).
            ["dyn-value-1", " x2", "x2", "old"]
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
        assert!(
            files
                .iter()
                .all(|f| !f.windows(12).any(|w| w == b"v-0123456789")),
            "never under the state"
        );
        assert!(runtime_dir(&state).join("J1.json").is_file(), "in /dev/shm");
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

    /// M2 (review of 5f0afd6): a value cut out of a known secret (a
    /// password a step took from a known DATABASE_URL into GITHUB_ENV) is
    /// a secret too, and so is one held by a value classified in the same
    /// step.
    #[test]
    fn a_part_of_a_known_secret_is_one() {
        let known: BTreeSet<String> = ["postgres://u:pa55word-x9@h/db".to_string()].into();
        // An earlier step cut the password out into GITHUB_ENV as DB_PASS.
        let set: BTreeSet<String> = ["DB_PASS".to_string()].into();
        let c = classify_with(
            &env(&[("DB_PASS", "pa55word-x9"), ("SHORT", "u")]),
            &Provenance::default(),
            &known,
            &set,
            &BTreeMap::new(),
        );
        assert_eq!(c.names, ["DB_PASS"]);
        assert!(c.strong.is_empty(), "a secret of its step, not of the job");
        let c = classify_with(
            &env(&[
                ("APP_DSN", "postgres://u:pa55word-x9@h/db"),
                ("DB_PASS", "pa55word-x9"),
            ]),
            &prov(&["APP_DSN"]),
            &BTreeSet::new(),
            &set,
            &BTreeMap::new(),
        );
        assert_eq!(c.names, ["APP_DSN", "DB_PASS"]);
        assert_eq!(mask_str("pw=pa55word-x9", &c.masks), "pw=***");
        // The workflow assigns it from a step's output.
        let mut p = prov(&["APP_DSN"]);
        p.derived.insert("DB_PASS".to_string());
        let c = classify(
            &env(&[
                ("APP_DSN", "postgres://u:pa55word-x9@h/db"),
                ("DB_PASS", "pa55word-x9"),
            ]),
            &p,
            &BTreeSet::new(),
        );
        assert_eq!(c.names, ["APP_DSN", "DB_PASS"]);
    }

    /// A variable the job image itself defines (the official python
    /// image's GPG_KEY, a public key fingerprint) is no secret by its name
    /// or shape; the same name with another value (the workflow's), or one
    /// the workflow maps from a secret, still is.
    #[test]
    fn the_image_s_own_variables_are_no_secrets_by_name() {
        let fpr = "7169605F62C751356D054A26A821E680E5FA6305";
        let image: BTreeMap<String, String> = [
            ("GPG_KEY".to_string(), fpr.to_string()),
            (
                "IMAGE_TOKEN".to_string(),
                "q8Zr2xLm4Np7Vt1Ks9Wd3Fh6Jb0Gc5Ye".to_string(),
            ),
        ]
        .into();
        let none = BTreeSet::new();
        let c = classify_with(
            &env(&[
                ("GPG_KEY", fpr),
                ("IMAGE_TOKEN", "q8Zr2xLm4Np7Vt1Ks9Wd3Fh6Jb0Gc5Ye"),
            ]),
            &Provenance::default(),
            &none,
            &none,
            &image,
        );
        assert!(c.names.is_empty(), "{:?}", c.names);
        // Without the image's definition: classified by name and shape.
        let c = classify(
            &env(&[
                ("GPG_KEY", fpr),
                ("IMAGE_TOKEN", "q8Zr2xLm4Np7Vt1Ks9Wd3Fh6Jb0Gc5Ye"),
            ]),
            &Provenance::default(),
            &none,
        );
        assert_eq!(c.names, ["GPG_KEY", "IMAGE_TOKEN"]);
        // The workflow overrides it: its value is not the image's.
        let c = classify_with(
            &env(&[("GPG_KEY", "another-key-0123456789")]),
            &Provenance::default(),
            &none,
            &none,
            &image,
        );
        assert_eq!(c.names, ["GPG_KEY"]);
        // Provenance wins: the workflow maps a secret onto it.
        let c = classify_with(
            &env(&[("GPG_KEY", fpr)]),
            &prov(&["GPG_KEY"]),
            &none,
            &none,
            &image,
        );
        assert_eq!(c.names, ["GPG_KEY"]);
        // So does a known secret of the job with that value.
        let known: BTreeSet<String> = [fpr.to_string()].into();
        let c = classify_with(
            &env(&[("GPG_KEY", fpr)]),
            &Provenance::default(),
            &known,
            &none,
            &image,
        );
        assert_eq!(c.names, ["GPG_KEY"]);
    }

    /// R1 (verification of 42c4498): configuration a structured secret
    /// happens to hold (a dotenv blob, a DSN) is no secret under a name the
    /// workflow or the runner sets: the reviewer's probe.
    #[test]
    fn configuration_inside_a_secret_is_not_one() {
        let known: BTreeSet<String> = [
            "NODE_ENV=production\nAPI_URL=https://api.example.com\nDB_PASSWORD=Zx9-very-secret-77\nREGION=us-east-1\nGITHUB_REPOSITORY=acme/webapp\n".to_string(),
            "postgres://postgres:pw123456@localhost:5432/postgres".to_string(),
        ]
        .into();
        let e = env(&[
            ("NODE_ENV", "production"),
            ("REGION", "us-east-1"),
            ("GITHUB_REPOSITORY", "acme/webapp"),
            ("PGHOST", "localhost"),
            ("PGUSER", "postgres"),
            ("SERVICE", "api.example.com"),
            ("CI", "true"),
        ]);
        let c = classify(&e, &Provenance::default(), &known);
        assert!(c.names.is_empty(), "{:?}", c.names);
        assert!(c.strong.is_empty());
        let line = "deploying to production in us-east-1 for acme/webapp";
        assert_eq!(mask_str(line, &c.masks), line);
        // In the same step as the DSN itself, under the workflow's names.
        let mut e2 = e.clone();
        e2.insert(
            "DATABASE_URL".into(),
            "postgres://postgres:pw123456@localhost:5432/postgres".into(),
        );
        let c = classify(&e2, &prov(&["DATABASE_URL"]), &BTreeSet::new());
        assert_eq!(c.names, ["DATABASE_URL"]);
    }

    /// M5 (verification of 42c4498): base64 wrapped at 64 or 76 columns
    /// with the secret at any offset and of any length: `echo "$S" |
    /// base64` (a trailing newline) for 100- and 101-byte secrets, and a
    /// `user:` prefix.
    #[test]
    fn wrapped_base64_at_any_offset() {
        let wrap = |s: &str, w: usize| -> String {
            s.as_bytes()
                .chunks(w)
                .map(|c| format!("{}\n", String::from_utf8_lossy(c)))
                .collect()
        };
        let base = "github_pat_11ABCDEFG0123456789_abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789abcdef";
        for sec in [format!("{base}g"), format!("{base}gh"), base.to_string()] {
            let m = masks_for(&sec);
            for input in [format!("{sec}\n"), format!("user:{sec}"), sec.clone()] {
                let enc = b64(input.as_bytes());
                for w in [64, 76] {
                    let out = String::from_utf8(mask(wrap(&enc, w).as_bytes(), &m)).unwrap();
                    let joined: String = out.lines().collect();
                    for win in enc.as_bytes().windows(12) {
                        let win = std::str::from_utf8(win).unwrap();
                        assert!(
                            !joined.contains(win),
                            "len {} {:?} at {w}: {win} left in {out}",
                            sec.len(),
                            &input[..5]
                        );
                    }
                    assert_eq!(out.lines().count(), wrap(&enc, w).lines().count());
                }
            }
        }
    }

    /// Final verification of f357bba: wrapped base64 indented (a YAML
    /// block, `sed 's/^/    /'`, a tab) or after a prefix on its first line
    /// (`echo "auth: $(base64 f)"`).
    #[test]
    fn indented_and_prefixed_wrapped_base64() {
        let base = "github_pat_11ABCDEFG0123456789_abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789abc";
        for sec in [base.to_string(), format!("{base}d"), format!("{base}de")] {
            let m = masks_for(&sec);
            let enc = b64(sec.as_bytes());
            for w in [64, 76] {
                let lines: Vec<String> = enc
                    .as_bytes()
                    .chunks(w)
                    .map(|c| String::from_utf8_lossy(c).into_owned())
                    .collect();
                let forms = [
                    lines.iter().map(|l| format!("  {l}\n")).collect::<String>(),
                    lines.iter().map(|l| format!("    {l}\n")).collect(),
                    lines.iter().map(|l| format!("\t{l}\n")).collect(),
                    format!("auth: {}\n", lines.join("\n")),
                    format!("data:\n  key: {}\n  other: x\n", lines.join("\n  ")),
                ];
                for text in forms {
                    assert!(holds(text.as_bytes(), &m), "{} at {w}: {text}", sec.len());
                    let out = String::from_utf8(mask(text.as_bytes(), &m)).unwrap();
                    let joined: String = out.split_whitespace().collect();
                    for win in enc.as_bytes().windows(12) {
                        let win = std::str::from_utf8(win).unwrap();
                        assert!(
                            !joined.contains(win),
                            "{} at {w}: {win} left in {out}",
                            sec.len()
                        );
                    }
                    assert_eq!(out.lines().count(), text.lines().count());
                }
            }
        }
    }

    /// M3 (review of 5f0afd6): `::add-mask::` as the runner parses it: the
    /// properties form, and `core.setSecret`'s escaping undone.
    #[test]
    fn add_mask_as_the_runner_reads_it() {
        assert_eq!(
            add_masks(b"::add-mask k=v::abc%25def%0Aghi-0123\n"),
            ["abc%def\nghi-0123"]
        );
        assert_eq!(add_masks(b"::ADD-MASK::x%0D\n"), ["x\r", "x"]);
        assert_eq!(add_masks(b"##[add-mask]a%3Bb%5Dc\n"), ["a;b]c"]);
    }

    /// M4 (review of 5f0afd6): a workflow command line holding a secret
    /// makes the run unrecordable; and a value flagged only by its name or
    /// shape is no certain secret of the job.
    #[test]
    fn command_lines_and_certain_secrets() {
        let m = masks_for("s3cr3t-DSN value");
        assert!(command_line_holds(
            b"x\n::set-output name=dsn::s3cr3t-DSN value\n",
            &m
        ));
        assert!(command_line_holds(
            b"  ##[set-output name=d;]s3cr3t-DSN value\n",
            &m
        ));
        assert!(!command_line_holds(b"dsn=s3cr3t-DSN value\n", &m));
        let c = classify(
            &env(&[
                ("GIT_AUTHOR_NAME", "github-actions[bot]"),
                ("APP_DSN", "s3cr3t-DSN value"),
                ("PW", "short"),
            ]),
            &prov(&["APP_DSN", "PW"]),
            &BTreeSet::new(),
        );
        assert!(
            c.is_secret("GIT_AUTHOR_NAME"),
            "still salted in its own step"
        );
        assert_eq!(
            c.strong.iter().collect::<Vec<_>>(),
            ["s3cr3t-DSN value"],
            "only certain values of 8 bytes or more join the job's secrets"
        );
        assert!(c.short_strong && c.short.as_deref() == Some("PW"));
    }

    /// M5 (review of 5f0afd6): base64 wrapped at 76 columns (GNU base64)
    /// or 64 (openssl, PEM) is masked line by line.
    #[test]
    fn wrapped_base64_is_masked() {
        let pem = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW\nQyNTUxOQAAACDx5kQ2bW9yZS1rZXktbWF0ZXJpYWwtMDEyMzQ1Njc4OQ==\n-----END OPENSSH PRIVATE KEY-----\n";
        let whole = b64(pem.as_bytes());
        for width in [64, 76] {
            let wrapped: String = whole
                .as_bytes()
                .chunks(width)
                .map(|c| format!("{}\n", String::from_utf8_lossy(c)))
                .collect();
            let out = masked(&wrapped, pem);
            for line in wrapped.lines().filter(|l| l.len() >= MIN_SECRET_LEN) {
                assert!(!out.contains(line), "width {width}: {line} left in {out}");
            }
        }
    }
}
