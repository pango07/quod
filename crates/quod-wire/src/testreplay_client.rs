//! The runner's side of test-level replay in Quod Cloud (`testreplay`):
//! look up and fetch a ledger directory, save one, post a run report.
//!
//! Same rule as `RemoteLedger`: on any doubt, nothing is used. A lookup
//! answer is used only when its signature verifies under the server key
//! the runner trusts (`QUOD_SERVER_KEY`, else `/v1/info` over TLS) and
//! names this lookup's nonce and exactly the files listed; every file is
//! then checked against its BLAKE3 as it arrives, and its path must stay
//! inside the ledger directory. The caller treats every error as "no
//! ledger" (every test file runs), never as a failed job.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use ed25519_dalek::VerifyingKey;
use quod_cas::Hash;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::client::{Auth, CallError, ServerConfig};
use crate::testreplay::*;
use crate::{BlobsRequest, BlobsResponse, Info, PROTOCOL, PROTOCOL_HEADER, verifying_key_from_hex};

/// Uploads in flight at once.
const UPLOADERS: usize = 8;

pub struct TestReplayClient {
    cfg: ServerConfig,
    agent: ureq::Agent,
    bearer: OnceLock<String>,
    key: OnceLock<VerifyingKey>,
}

/// A nonce: 16 bytes from the OS's randomness.
fn nonce() -> Result<String, CallError> {
    let mut r = [0u8; 16];
    getrandom::getrandom(&mut r).map_err(|e| CallError(format!("randomness: {e}")))?;
    Ok(hex::encode(r))
}

impl TestReplayClient {
    pub fn new(cfg: ServerConfig) -> TestReplayClient {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(5).min(cfg.timeout))
            .timeout(cfg.timeout)
            .build();
        TestReplayClient {
            key: OnceLock::new(),
            bearer: OnceLock::new(),
            cfg,
            agent,
        }
    }

    pub fn url(&self) -> &str {
        &self.cfg.url
    }

    fn bearer(&self) -> Result<String, CallError> {
        if let Some(b) = self.bearer.get() {
            return Ok(b.clone());
        }
        let b = match &self.cfg.auth {
            Auth::Token(t) => t.clone(),
            Auth::GithubOidc {
                request_url,
                request_token,
                audience,
            } => {
                let sep = if request_url.contains('?') { '&' } else { '?' };
                let v: serde_json::Value = self
                    .agent
                    .get(&format!("{request_url}{sep}audience={audience}"))
                    .set("authorization", &format!("bearer {request_token}"))
                    .call()?
                    .into_json()?;
                v["value"]
                    .as_str()
                    .ok_or_else(|| CallError("the OIDC token endpoint returned no value".into()))?
                    .to_string()
            }
        };
        let _ = self.bearer.set(b.clone());
        Ok(b)
    }

    fn request(&self, method: &str, path: &str) -> Result<ureq::Request, CallError> {
        Ok(self
            .agent
            .request(method, &format!("{}{path}", self.cfg.url))
            .set(PROTOCOL_HEADER, PROTOCOL)
            .set("authorization", &format!("Bearer {}", self.bearer()?)))
    }

    fn post<T: DeserializeOwned>(&self, path: &str, body: &impl Serialize) -> Result<T, CallError> {
        let body = serde_json::to_vec(body).map_err(|e| CallError(e.to_string()))?;
        let resp = self
            .request("POST", path)?
            .set("content-type", "application/json")
            .send_bytes(&body)?;
        if resp.header(PROTOCOL_HEADER) != Some(PROTOCOL) {
            return Err(CallError("the answer is not quod.v1".into()));
        }
        resp.into_json()
            .map_err(|e| CallError(format!("bad answer: {e}")))
    }

    /// The key lookups must be signed with: the pinned one, else the one
    /// the server serves over TLS (with a notice saying how to pin it).
    fn server_key(&self) -> Result<VerifyingKey, CallError> {
        if let Some(k) = self.cfg.server_key {
            return Ok(k);
        }
        if let Some(k) = self.key.get() {
            return Ok(*k);
        }
        let info: Info = self
            .agent
            .get(&format!("{}/v1/info", self.cfg.url))
            .set(PROTOCOL_HEADER, PROTOCOL)
            .call()?
            .into_json()?;
        let k = verifying_key_from_hex(&info.signing_key).map_err(|e| CallError(e.to_string()))?;
        eprintln!(
            "quod: QUOD_SERVER_KEY is not set; trusting the key {} serves over TLS ({}). Pin it: QUOD_SERVER_KEY={}",
            self.cfg.url,
            crate::key_fingerprint(&k),
            hex::encode(k.to_bytes())
        );
        let _ = self.key.set(k);
        Ok(k)
    }

    /// The ledger to start from, verified for this job (`me`), or none.
    pub fn lookup(
        &self,
        suite: &str,
        ancestors: &[String],
        me: &Requester,
    ) -> Result<LookupResponse, CallError> {
        let nonce = nonce()?;
        let resp: LookupResponse = self.post(
            "/v1/test-ledgers/lookup",
            &LookupRequest {
                suite: suite.into(),
                ancestors: ancestors.iter().take(MAX_ANCESTORS).cloned().collect(),
                nonce: nonce.clone(),
                ci: self.cfg.ci.clone(),
                scope: self.cfg.scope.clone(),
            },
        )?;
        if let Some(f) = &resp.ledger {
            verify_ledger(&self.server_key()?, f, suite, &nonce, me)
                .map_err(|e| CallError(format!("the ledger is not trusted: {e}")))?;
            check_files(&f.files).map_err(|e| CallError(format!("the ledger's files: {e}")))?;
        }
        Ok(resp)
    }

    /// Fetches `files` into `dest` (an empty directory), each checked
    /// against its hash: from the ledger's bundle (one object) when the
    /// lookup named one, else (or when the bundle fails) with a pack.
    /// Returns the bytes fetched and how (`bundle` or `pack`).
    pub fn fetch(
        &self,
        files: &[LedgerFile],
        dest: &Path,
        bundle: Option<&BundleRef>,
    ) -> Result<(u64, &'static str), CallError> {
        check_files(files).map_err(CallError)?;
        let mut by_hash: BTreeMap<Hash, Vec<&LedgerFile>> = BTreeMap::new();
        for f in files {
            by_hash.entry(f.hash).or_default().push(f);
        }
        if let Some(b) = bundle {
            let got = (|| -> Result<u64, CallError> {
                let resp = self
                    .request("GET", &format!("/v1/blobs/{}", b.hash.hex()))?
                    .call()?;
                // The bundle's own hash is checked as it streams, and every
                // file in it against the signed list.
                let mut r = BufReader::with_capacity(
                    1 << 20,
                    HashingReader {
                        inner: resp.into_reader(),
                        hasher: blake3::Hasher::new(),
                    },
                );
                let n = read_entries(&mut r, &by_hash, dest, None)?;
                let mut rest = Vec::new();
                r.read_to_end(&mut rest)?;
                if !rest.is_empty() || Hash::from_blake3(r.into_inner().hasher.finalize()) != b.hash
                {
                    return Err(CallError("the bundle is not what its name says".into()));
                }
                Ok(n)
            })();
            match got {
                Ok(n) => return Ok((n, "bundle")),
                Err(e) => {
                    tracing::warn!(target: "quod", error = %e.0, "ledger bundle not used; fetching file by file");
                    let _ = std::fs::remove_dir_all(dest);
                    std::fs::create_dir_all(dest)?;
                }
            }
        }
        let hashes: Vec<Hash> = by_hash.keys().copied().collect();
        let mut total = 0u64;
        for chunk in hashes.chunks(MAX_PACK) {
            let resp = self
                .request("POST", "/v1/blobs/pack")?
                .set("content-type", "application/json")
                .send_bytes(
                    &serde_json::to_vec(&PackRequest {
                        hashes: chunk.to_vec(),
                    })
                    .map_err(|e| CallError(e.to_string()))?,
                )?;
            let mut r = BufReader::with_capacity(1 << 20, resp.into_reader());
            let sub: BTreeMap<Hash, Vec<&LedgerFile>> =
                chunk.iter().map(|h| (*h, by_hash[h].clone())).collect();
            total += read_entries(&mut r, &sub, dest, Some(chunk))?;
        }
        Ok((total, "pack"))
    }

    /// Saves the ledger directory `dir`: uploads the files the server
    /// lacks, then the list. Returns the server's answer.
    pub fn save(&self, suite: &str, dir: &Path) -> Result<(SaveResponse, usize, u64), CallError> {
        let files = ledger_files(dir)?;
        let list: Vec<LedgerFile> = files.iter().map(|(f, _)| f.clone()).collect();
        check_files(&list).map_err(CallError)?;
        let mut hashes: Vec<Hash> = list.iter().map(|f| f.hash).collect();
        hashes.sort();
        hashes.dedup();
        let missing: BlobsResponse = self.post("/v1/blobs/missing", &BlobsRequest { hashes })?;
        let path_of: BTreeMap<Hash, &PathBuf> = files.iter().map(|(f, p)| (f.hash, p)).collect();
        let todo: Vec<(&Hash, &PathBuf)> = missing
            .missing
            .iter()
            .filter_map(|h| path_of.get(h).map(|p| (h, *p)))
            .collect();
        let next = AtomicUsize::new(0);
        let sent = std::sync::atomic::AtomicU64::new(0);
        let errors: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(vec![]);
        // The bearer is fetched once, before the threads share it.
        self.bearer()?;
        std::thread::scope(|s| {
            for _ in 0..UPLOADERS.min(todo.len()) {
                s.spawn(|| {
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some((h, p)) = todo.get(i) else { break };
                        let r = (|| -> Result<u64, CallError> {
                            let size = std::fs::metadata(p)?.len();
                            let f = std::fs::File::open(p)?;
                            self.request("PUT", &format!("/v1/blobs/{}", h.hex()))?
                                .set("content-type", "application/octet-stream")
                                .set("content-length", &size.to_string())
                                .send(f)?;
                            Ok(size)
                        })();
                        match r {
                            Ok(n) => {
                                sent.fetch_add(n, Ordering::Relaxed);
                            }
                            Err(e) => {
                                errors.lock().unwrap().push(format!("{h}: {}", e.0));
                                break;
                            }
                        }
                    }
                });
            }
        });
        if let Some(e) = errors.into_inner().unwrap().into_iter().next() {
            return Err(CallError(format!("upload failed: {e}")));
        }
        let resp: SaveResponse = self.post(
            "/v1/test-ledgers",
            &SaveRequest {
                suite: suite.into(),
                files: list,
                ci: self.cfg.ci.clone(),
                scope: self.cfg.scope.clone(),
            },
        )?;
        Ok((resp, todo.len(), sent.into_inner()))
    }

    pub fn report(&self, suite: &str, report: &TestReport) -> Result<ReportResponse, CallError> {
        self.post(
            "/v1/test-runs",
            &ReportRequest {
                suite: suite.into(),
                report: report.clone(),
                ci: self.cfg.ci.clone(),
                scope: self.cfg.scope.clone(),
            },
        )
    }
}

/// A reader that hashes what passes through it.
struct HashingReader<R> {
    inner: R,
    hasher: blake3::Hasher,
}

impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.hasher.update(&buf[..n]);
        Ok(n)
    }
}

/// Reads pack-format entries (`<64 hex> <size>\n` and the bytes, or
/// `<64 hex> -\n`) into `dest`: each blob's files, created new, checked
/// against the hash. `order`: the hashes in the order the server must send
/// them (a pack); `None`: any order, each wanted hash exactly once (a
/// bundle). Returns the bytes written (once per blob).
fn read_entries(
    r: &mut impl BufRead,
    by_hash: &BTreeMap<Hash, Vec<&LedgerFile>>,
    dest: &Path,
    order: Option<&[Hash]>,
) -> Result<u64, CallError> {
    let mut seen = std::collections::BTreeSet::new();
    let mut total = 0u64;
    for k in 0..by_hash.len() {
        let mut line = Vec::new();
        (&mut *r).take(200).read_until(b'\n', &mut line)?;
        let line = String::from_utf8(line).map_err(|_| CallError("a bad pack header".into()))?;
        let (h, size) = line
            .trim_end()
            .split_once(' ')
            .ok_or_else(|| CallError("a bad pack header".into()))?;
        let want: Hash = format!("b3:{h}")
            .parse()
            .map_err(|_| CallError("a bad pack header".into()))?;
        if let Some(o) = order
            && o.get(k) != Some(&want)
        {
            return Err(CallError("the pack is out of order".into()));
        }
        let Some(targets) = by_hash.get(&want) else {
            return Err(CallError(format!("{want} is not in the ledger")));
        };
        if !seen.insert(want) {
            return Err(CallError(format!("{want} came twice")));
        }
        if size == "-" {
            return Err(CallError(format!("the server lacks {want}")));
        }
        let size: u64 = size
            .parse()
            .map_err(|_| CallError("a bad pack size".into()))?;
        if size != targets[0].size {
            return Err(CallError(format!("{want} has another size than listed")));
        }
        let first = dest.join(&targets[0].path);
        std::fs::create_dir_all(first.parent().unwrap_or(dest))?;
        // create_new: never through a link, never over a file already
        // written (two paths one file system folds together).
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&first)?;
        let mut hasher = blake3::Hasher::new();
        let mut left = size;
        let mut buf = vec![0u8; 1 << 16];
        while left > 0 {
            let n = r.read(&mut buf[..(left.min(1 << 16)) as usize])?;
            if n == 0 {
                return Err(CallError("the pack ended early".into()));
            }
            hasher.update(&buf[..n]);
            out.write_all(&buf[..n])?;
            left -= n as u64;
        }
        if Hash::from_blake3(hasher.finalize()) != want {
            return Err(CallError(format!("{want} came back with other bytes")));
        }
        total += size;
        for t in &targets[1..] {
            let p = dest.join(&t.path);
            std::fs::create_dir_all(p.parent().unwrap_or(dest))?;
            let mut o = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&p)?;
            std::io::copy(&mut std::fs::File::open(&first)?, &mut o)?;
        }
    }
    Ok(total)
}

/// The files of a ledger directory that travel (`TOP_FILES`, `TOP_DIRS`),
/// hashed, with their paths on disk. Symlinks and other non-regular files
/// are left out.
pub fn ledger_files(dir: &Path) -> std::io::Result<Vec<(LedgerFile, PathBuf)>> {
    let mut out = vec![];
    let mut stack: Vec<(PathBuf, String)> = vec![];
    for f in TOP_FILES {
        stack.push((dir.join(f), (*f).to_string()));
    }
    for d in TOP_DIRS {
        stack.push((dir.join(d), (*d).to_string()));
    }
    while let Some((p, rel)) = stack.pop() {
        let Ok(m) = std::fs::symlink_metadata(&p) else {
            continue;
        };
        if m.is_dir() {
            for e in std::fs::read_dir(&p)? {
                let e = e?;
                let Some(name) = e.file_name().to_str().map(str::to_string) else {
                    continue;
                };
                stack.push((e.path(), format!("{rel}/{name}")));
            }
        } else if m.is_file() {
            if !valid_path(&rel) || rel.ends_with(".tmp") {
                continue;
            }
            let mut h = blake3::Hasher::new();
            h.update_reader(std::fs::File::open(&p)?)?;
            out.push((
                LedgerFile {
                    path: rel,
                    hash: Hash::from_blake3(h.finalize()),
                    size: m.len(),
                },
                p,
            ));
        }
    }
    out.sort_by(|a, b| a.0.path.cmp(&b.0.path));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_travelling_files_are_listed() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        for (p, b) in [
            ("ledger.json", "{}"),
            ("results/a.json", "a"),
            ("blobs/x", "x"),
            ("render/stdout", "o"),
            ("plan.json", "p"),
            ("out.json", "o"),
            ("code/driver.mjs", "d"),
            ("dump/1", "1"),
            ("results/b.json.tmp", "t"),
        ] {
            let f = d.join(p);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, b).unwrap();
        }
        let got: Vec<String> = ledger_files(d)
            .unwrap()
            .into_iter()
            .map(|(f, _)| f.path)
            .collect();
        assert_eq!(
            got,
            ["blobs/x", "ledger.json", "render/stdout", "results/a.json"]
        );
        let l = ledger_files(d).unwrap();
        assert_eq!(l[1].0.hash, Hash::of(b"{}"));
    }
}
