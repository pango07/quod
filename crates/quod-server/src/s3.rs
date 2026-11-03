//! Quod Cloud's blobs in an S3-compatible bucket (P4, docs/cloud-ledger.md).
//!
//! A small blocking S3 client (AWS Signature Version 4 over ureq: HEAD,
//! GET, PUT and multipart uploads, nothing else) and `S3Blobs`, one team's
//! `BlobStore` on it. Objects are `<prefix>teams/<team>/blobs/<hex>`: a
//! team's blobs live under its own prefix, and no request names another
//! team's.
//!
//! Content addressing is kept as the local CAS keeps it: an object's name
//! is the BLAKE3 of exactly the bytes stored. Bytes in memory are hashed
//! before the PUT; a file is hashed once to name the object and again as
//! its parts go up, and the multipart upload is completed only when the
//! two agree (otherwise it is aborted and nothing is stored). Every PUT and
//! part is signed over its SHA-256, so the store refuses bytes changed in
//! flight. Reads are checked too: `read_blob` and `open_blob_reader` fail
//! when the bytes do not hash to the name they were read under.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use hmac::{Hmac, Mac};
use quod_cas::{BlobStore, CasError, Hash};
use sha2::{Digest, Sha256};

/// Parts of a multipart upload, and the largest blob sent in one PUT.
pub const PART_SIZE: usize = 16 << 20;
/// S3's smallest part (all but the last).
pub const MIN_PART_SIZE: usize = 5 << 20;

#[derive(Clone)]
pub struct S3Settings {
    /// `https://host[:port]` (or `http://` for a local MinIO).
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    /// Path-style URLs (`<endpoint>/<bucket>/<key>`) instead of
    /// virtual-hosted ones (`<bucket>.<host>/<key>`).
    pub path_style: bool,
    /// Prepended to every object name (tests; a shared bucket).
    pub prefix: String,
    pub part_size: usize,
}

impl std::fmt::Debug for S3Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3Settings")
            .field("endpoint", &self.endpoint)
            .field("bucket", &self.bucket)
            .field("region", &self.region)
            .field("path_style", &self.path_style)
            .field("prefix", &self.prefix)
            .finish_non_exhaustive()
    }
}

/// The client: settings and one connection pool.
#[derive(Clone)]
pub struct S3 {
    s: Arc<S3Settings>,
    scheme: &'static str,
    /// The endpoint's host and port as the Host header carries them.
    host: String,
    agent: ureq::Agent,
}

fn store_err(m: impl std::fmt::Display) -> CasError {
    CasError::Store(m.to_string())
}

type HmacSha256 = Hmac<Sha256>;

fn hmac(key: &[u8], data: &str) -> Vec<u8> {
    let mut m = HmacSha256::new_from_slice(key).expect("HMAC takes any key");
    m.update(data.as_bytes());
    m.finalize().into_bytes().to_vec()
}

fn sha256_hex(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

/// RFC 3986 unreserved characters as they are; everything else %XX
/// (`/` too unless `keep_slash`), as SigV4 canonicalizes.
fn uri_encode(s: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric()
            || matches!(b, b'-' | b'_' | b'.' | b'~')
            || (keep_slash && b == b'/')
        {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `(YYYYMMDD'T'HHMMSS'Z', YYYYMMDD)` for now.
fn amz_now() -> (String, String) {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock before 1970");
    let t = quod_ledger::format_rfc3339(d.as_secs(), 0);
    // 2026-11-02T10:48:00.000Z
    let date = format!("{}{}{}", &t[0..4], &t[5..7], &t[8..10]);
    (
        format!("{date}T{}{}{}Z", &t[11..13], &t[14..16], &t[17..19]),
        date,
    )
}

/// An object's bytes as the store sends them.
pub type ObjectReader = Box<dyn Read + Send + Sync>;

/// One request's answer: status, headers we need, body.
struct Answer {
    status: u16,
    etag: Option<String>,
    length: Option<u64>,
    body: Option<ObjectReader>,
}

impl S3 {
    pub fn new(s: S3Settings) -> Result<S3, String> {
        let (scheme, rest) = if let Some(r) = s.endpoint.strip_prefix("https://") {
            ("https", r)
        } else if let Some(r) = s.endpoint.strip_prefix("http://") {
            ("http", r)
        } else {
            return Err(format!(
                "S3_ENDPOINT {:?} must start with https:// (or http:// for a local server)",
                s.endpoint
            ));
        };
        let rest = rest.trim_end_matches('/');
        if rest.is_empty() || rest.contains('/') || rest.contains('@') {
            return Err(format!(
                "S3_ENDPOINT {:?} must be a scheme and a host (and port), with no path",
                s.endpoint
            ));
        }
        // The Host header ureq sends: no port when it is the scheme's own.
        let host = match (scheme, rest.rsplit_once(':')) {
            ("https", Some((h, "443"))) | ("http", Some((h, "80"))) => h.to_string(),
            _ => rest.to_string(),
        };
        if s.bucket.is_empty() || s.bucket.contains('/') {
            return Err(format!("S3_BUCKET {:?} is not a bucket name", s.bucket));
        }
        if s.part_size < MIN_PART_SIZE {
            return Err(format!(
                "the S3 part size must be at least {MIN_PART_SIZE} bytes"
            ));
        }
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(10))
            .timeout_read(Duration::from_secs(120))
            .timeout_write(Duration::from_secs(120))
            .build();
        Ok(S3 {
            s: Arc::new(s),
            scheme,
            host,
            agent,
        })
    }

    pub fn settings(&self) -> &S3Settings {
        &self.s
    }

    /// The object name for `key` under the configured prefix.
    pub fn object(&self, key: &str) -> String {
        format!("{}{key}", self.s.prefix)
    }

    /// (host, canonical path) of an object, or of the bucket for `None`.
    fn target(&self, object: Option<&str>) -> (String, String) {
        let key = object.map(|k| uri_encode(k, true)).unwrap_or_default();
        if self.s.path_style {
            let path = if key.is_empty() {
                format!("/{}", uri_encode(&self.s.bucket, false))
            } else {
                format!("/{}/{key}", uri_encode(&self.s.bucket, false))
            };
            (self.host.clone(), path)
        } else {
            (
                format!("{}.{}", self.s.bucket, self.host),
                format!("/{key}"),
            )
        }
    }

    /// Sends one signed request, retrying a transport failure or a 5xx up
    /// to twice (every request here is idempotent).
    fn send(
        &self,
        method: &str,
        object: Option<&str>,
        query: &[(&str, &str)],
        body: &[u8],
    ) -> Result<Answer, CasError> {
        let mut last = String::new();
        for attempt in 0..3 {
            if attempt > 0 {
                std::thread::sleep(Duration::from_millis(250 * 4u64.pow(attempt - 1)));
            }
            match self.send_once(method, object, query, body) {
                Ok(a) if a.status >= 500 => {
                    last = format!("{method} answered {}", a.status);
                }
                Ok(a) => return Ok(a),
                Err(e) => last = e,
            }
        }
        Err(store_err(format!(
            "object store {method} {}: {last}",
            object.unwrap_or("(bucket)")
        )))
    }

    fn send_once(
        &self,
        method: &str,
        object: Option<&str>,
        query: &[(&str, &str)],
        body: &[u8],
    ) -> Result<Answer, String> {
        let (host, path) = self.target(object);
        let mut q: Vec<(String, String)> = query
            .iter()
            .map(|(k, v)| (uri_encode(k, false), uri_encode(v, false)))
            .collect();
        q.sort();
        let canonical_query = q
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&");
        let payload = sha256_hex(body);
        let (amz_date, date) = amz_now();
        let canonical_request = format!(
            "{method}\n{path}\n{canonical_query}\nhost:{host}\nx-amz-content-sha256:{payload}\nx-amz-date:{amz_date}\n\nhost;x-amz-content-sha256;x-amz-date\n{payload}",
        );
        let scope = format!("{date}/{}/s3/aws4_request", self.s.region);
        let to_sign = format!(
            "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
            sha256_hex(canonical_request.as_bytes())
        );
        let k = hmac(
            format!("AWS4{}", self.s.secret_access_key).as_bytes(),
            &date,
        );
        let k = hmac(&k, &self.s.region);
        let k = hmac(&k, "s3");
        let k = hmac(&k, "aws4_request");
        let signature = hex::encode(hmac(&k, &to_sign));
        let auth = format!(
            "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders=host;x-amz-content-sha256;x-amz-date, Signature={signature}",
            self.s.access_key_id
        );
        let mut url = format!("{}://{host}{path}", self.scheme);
        if !canonical_query.is_empty() {
            url.push('?');
            url.push_str(&canonical_query);
        }
        let req = self
            .agent
            .request(method, &url)
            .set("host", &host)
            .set("x-amz-date", &amz_date)
            .set("x-amz-content-sha256", &payload)
            .set("authorization", &auth);
        let resp = match if method == "PUT" || method == "POST" {
            req.send_bytes(body)
        } else {
            req.call()
        } {
            Ok(r) => r,
            Err(ureq::Error::Status(_, r)) => r,
            Err(e) => return Err(e.to_string()),
        };
        let status = resp.status();
        let etag = resp.header("etag").map(str::to_string);
        let length = resp.header("content-length").and_then(|v| v.parse().ok());
        Ok(Answer {
            status,
            etag,
            length,
            body: if method == "HEAD" {
                None
            } else {
                Some(resp.into_reader())
            },
        })
    }

    fn fail(what: &str, a: Answer) -> CasError {
        let mut text = String::new();
        if let Some(mut b) = a.body {
            let _ = b.by_ref().take(4096).read_to_string(&mut text);
        }
        store_err(format!("object store {what}: {} {}", a.status, text.trim()))
    }

    /// An object's size, or None when there is none.
    pub fn head(&self, object: &str) -> Result<Option<u64>, CasError> {
        let a = self.send("HEAD", Some(object), &[], &[])?;
        match a.status {
            200 => Ok(Some(a.length.unwrap_or(0))),
            404 => Ok(None),
            _ => Err(Self::fail("HEAD", a)),
        }
    }

    /// An object's bytes as a reader, or None when there is none.
    pub fn get(&self, object: &str) -> Result<Option<(ObjectReader, Option<u64>)>, CasError> {
        let a = self.send("GET", Some(object), &[], &[])?;
        match a.status {
            200 => Ok(Some((a.body.expect("GET has a body"), a.length))),
            404 => Ok(None),
            _ => Err(Self::fail("GET", a)),
        }
    }

    pub fn put(&self, object: &str, bytes: &[u8]) -> Result<(), CasError> {
        let a = self.send("PUT", Some(object), &[], bytes)?;
        match a.status {
            200 | 201 | 204 => Ok(()),
            _ => Err(Self::fail("PUT", a)),
        }
    }

    /// Creates the bucket (tests against a fresh MinIO); an existing one
    /// is fine.
    pub fn create_bucket(&self) -> Result<(), CasError> {
        let a = self.send("PUT", None, &[], &[])?;
        match a.status {
            200 | 204 | 409 => Ok(()),
            _ => Err(Self::fail("create bucket", a)),
        }
    }

    fn body_text(a: Answer) -> Result<String, CasError> {
        let mut s = String::new();
        if let Some(mut b) = a.body {
            b.by_ref()
                .take(1 << 20)
                .read_to_string(&mut s)
                .map_err(store_err)?;
        }
        Ok(s)
    }

    fn create_multipart(&self, object: &str) -> Result<String, CasError> {
        let a = self.send("POST", Some(object), &[("uploads", "")], &[])?;
        if a.status != 200 {
            return Err(Self::fail("create multipart upload", a));
        }
        let text = Self::body_text(a)?;
        xml_value(&text, "UploadId")
            .ok_or_else(|| store_err(format!("object store: no UploadId in {text:?}")))
    }

    fn upload_part(
        &self,
        object: &str,
        id: &str,
        n: usize,
        bytes: &[u8],
    ) -> Result<String, CasError> {
        let n = n.to_string();
        let a = self.send(
            "PUT",
            Some(object),
            &[("partNumber", &n), ("uploadId", id)],
            bytes,
        )?;
        if a.status != 200 {
            return Err(Self::fail("upload part", a));
        }
        a.etag
            .ok_or_else(|| store_err("object store: a part answered without an ETag"))
    }

    fn complete_multipart(&self, object: &str, id: &str, etags: &[String]) -> Result<(), CasError> {
        let mut xml = String::from("<CompleteMultipartUpload>");
        for (i, e) in etags.iter().enumerate() {
            xml.push_str(&format!(
                "<Part><PartNumber>{}</PartNumber><ETag>{}</ETag></Part>",
                i + 1,
                e.replace('&', "&amp;").replace('<', "&lt;")
            ));
        }
        xml.push_str("</CompleteMultipartUpload>");
        let a = self.send("POST", Some(object), &[("uploadId", id)], xml.as_bytes())?;
        if a.status != 200 {
            return Err(Self::fail("complete multipart upload", a));
        }
        // S3 may answer 200 and put the error in the body.
        let text = Self::body_text(a)?;
        if text.contains("<Error>") {
            return Err(store_err(format!(
                "object store: complete multipart upload: {}",
                text.trim()
            )));
        }
        Ok(())
    }

    fn abort_multipart(&self, object: &str, id: &str) {
        let _ = self.send("DELETE", Some(object), &[("uploadId", id)], &[]);
    }

    /// Uploads `read`'s bytes to `object` in parts and completes the
    /// upload only if `check` accepts the hash and length of what was sent
    /// (aborted otherwise: nothing is stored).
    fn put_multipart(
        &self,
        object: &str,
        read: &mut dyn Read,
        check: impl FnOnce(Hash, u64) -> Result<(), CasError>,
    ) -> Result<u64, CasError> {
        let id = self.create_multipart(object)?;
        let out = (|| {
            let mut hasher = blake3::Hasher::new();
            let mut etags = vec![];
            let mut total = 0u64;
            let mut buf = vec![0u8; self.s.part_size];
            loop {
                let n = read_full(read, &mut buf).map_err(store_err)?;
                if n == 0 && !etags.is_empty() {
                    break;
                }
                hasher.update(&buf[..n]);
                total += n as u64;
                etags.push(self.upload_part(object, &id, etags.len() + 1, &buf[..n])?);
                if n < buf.len() {
                    break;
                }
            }
            check(Hash::from_blake3(hasher.finalize()), total)?;
            self.complete_multipart(object, &id, &etags)?;
            Ok(total)
        })();
        if out.is_err() {
            self.abort_multipart(object, &id);
        }
        out
    }
}

/// Reads until `buf` is full or the reader ends; how many bytes.
fn read_full(r: &mut dyn Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}

/// The text of the first `<tag>` element.
fn xml_value(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = start + xml[start..].find(&format!("</{tag}>"))?;
    Some(xml[start..end].to_string())
}

/// A reader that fails at the end when the bytes it passed on do not hash
/// to `want`.
pub struct Verified<R> {
    inner: R,
    hasher: blake3::Hasher,
    want: Hash,
}

impl<R: Read> Read for Verified<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n == 0 && !buf.is_empty() {
            let got = Hash::from_blake3(self.hasher.finalize());
            if got != self.want {
                return Err(std::io::Error::other(format!(
                    "the object store's bytes for {} hash to {got}",
                    self.want
                )));
            }
        }
        self.hasher.update(&buf[..n]);
        Ok(n)
    }
}

/// One team's blobs in the bucket.
#[derive(Clone)]
pub struct S3Blobs {
    s3: S3,
    team: String,
}

impl S3Blobs {
    pub fn new(s3: S3, team: &str) -> S3Blobs {
        debug_assert!(crate::valid_id(team));
        S3Blobs {
            s3,
            team: team.to_string(),
        }
    }

    /// The object holding blob `h` of this team.
    pub fn object(&self, h: &Hash) -> String {
        self.s3
            .object(&format!("teams/{}/blobs/{}", self.team, h.hex()))
    }

    /// The blob's size, or None when the team has no such blob.
    pub fn size(&self, h: &Hash) -> Result<Option<u64>, CasError> {
        self.s3.head(&self.object(h))
    }

    /// The blob as a reader that fails at its end unless the bytes hash to
    /// `h`, and its length when the store said.
    pub fn open(&self, h: &Hash) -> Result<(Verified<ObjectReader>, Option<u64>), CasError> {
        let (r, len) = self.s3.get(&self.object(h))?.ok_or(CasError::Missing(*h))?;
        Ok((
            Verified {
                inner: r,
                hasher: blake3::Hasher::new(),
                want: *h,
            },
            len,
        ))
    }

    fn put_bytes(&self, h: &Hash, bytes: &[u8]) -> Result<(), CasError> {
        let obj = self.object(h);
        if bytes.len() <= self.s3.s.part_size {
            return self.s3.put(&obj, bytes);
        }
        let want = *h;
        self.s3
            .put_multipart(&obj, &mut &bytes[..], |got, _| {
                if got == want {
                    Ok(())
                } else {
                    Err(store_err("bytes changed while being uploaded"))
                }
            })
            .map(|_| ())
    }
}

impl BlobStore for S3Blobs {
    fn has_blob(&self, h: &Hash) -> Result<bool, CasError> {
        Ok(self.size(h)?.is_some())
    }

    fn read_blob(&self, h: &Hash) -> Result<Vec<u8>, CasError> {
        let (mut r, len) = self.open(h)?;
        let mut buf = Vec::with_capacity(len.unwrap_or(0).min(64 << 20) as usize);
        r.read_to_end(&mut buf).map_err(store_err)?;
        Ok(buf)
    }

    fn open_blob_reader(&self, h: &Hash) -> Result<Box<dyn Read + Send>, CasError> {
        Ok(Box::new(self.open(h)?.0))
    }

    fn put_blob(&self, bytes: &[u8]) -> Result<Hash, CasError> {
        let h = Hash::of(bytes);
        if !self.has_blob(&h)? {
            self.put_bytes(&h, bytes)?;
        }
        Ok(h)
    }

    /// Hashes the file to name the object, then uploads it, hashing again
    /// as it goes: a file that changed in between is refused
    /// (`CasError::Changed`) and nothing is stored.
    fn put_blob_file(&self, path: &Path) -> Result<(Hash, u64), CasError> {
        let io = |e: std::io::Error| CasError::Io {
            path: path.to_path_buf(),
            source: e,
        };
        let (h, len) = {
            let mut f = std::fs::File::open(path).map_err(io)?;
            let mut hasher = blake3::Hasher::new();
            let n = std::io::copy(&mut f, &mut HashWriter(&mut hasher)).map_err(io)?;
            (Hash::from_blake3(hasher.finalize()), n)
        };
        if self.has_blob(&h)? {
            return Ok((h, 0));
        }
        let mut f = std::fs::File::open(path).map_err(io)?;
        if len as usize <= self.s3.s.part_size {
            let mut bytes = Vec::with_capacity(len as usize);
            f.read_to_end(&mut bytes).map_err(io)?;
            if Hash::of(&bytes) != h {
                return Err(CasError::Changed(path.to_path_buf()));
            }
            self.s3.put(&self.object(&h), &bytes)?;
            return Ok((h, len));
        }
        let changed = path.to_path_buf();
        let n = self.s3.put_multipart(&self.object(&h), &mut f, |got, n| {
            if got == h && n == len {
                Ok(())
            } else {
                Err(CasError::Changed(changed))
            }
        })?;
        Ok((h, n))
    }
}

struct HashWriter<'a>(&'a mut blake3::Hasher);

impl Write for HashWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.update(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sigv4_canonical_pieces() {
        assert_eq!(
            uri_encode("teams/acme/blobs/ab", true),
            "teams/acme/blobs/ab"
        );
        assert_eq!(uri_encode("a b/c", false), "a%20b%2Fc");
        assert_eq!(uri_encode("~x_y.z-", false), "~x_y.z-");
        let (d, day) = amz_now();
        assert_eq!(d.len(), 16);
        assert!(d.starts_with(&day) && d.ends_with('Z'));
        assert_eq!(
            xml_value("<a><UploadId>x1</UploadId></a>", "UploadId").as_deref(),
            Some("x1")
        );
    }

    /// AWS's published example of the signing key derivation
    /// (docs.aws.amazon.com, "Examples of how to derive a signing key").
    #[test]
    fn signing_key_matches_aws_example() {
        let k = hmac(b"AWS4wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY", "20120215");
        let k = hmac(&k, "us-east-1");
        let k = hmac(&k, "iam");
        let k = hmac(&k, "aws4_request");
        assert_eq!(
            hex::encode(k),
            "f4780e2d9f65fa895f9c67b32ce1baf0b0d8a43505a000a1a9e090d414db404d"
        );
    }

    #[test]
    fn endpoints_hosts_and_styles() {
        let s = |endpoint: &str, path_style: bool| S3Settings {
            endpoint: endpoint.into(),
            bucket: "quod-results-abc".into(),
            region: "auto".into(),
            access_key_id: "k".into(),
            secret_access_key: "s".into(),
            path_style,
            prefix: String::new(),
            part_size: PART_SIZE,
        };
        let v = S3::new(s("https://t3.storageapi.dev", false)).unwrap();
        assert_eq!(
            v.target(Some("teams/a/blobs/x")),
            (
                "quod-results-abc.t3.storageapi.dev".into(),
                "/teams/a/blobs/x".into()
            )
        );
        let p = S3::new(s("http://127.0.0.1:9000/", true)).unwrap();
        assert_eq!(
            p.target(Some("k")),
            ("127.0.0.1:9000".into(), "/quod-results-abc/k".into())
        );
        assert_eq!(S3::new(s("https://h:443", true)).unwrap().host, "h");
        assert!(S3::new(s("t3.storageapi.dev", false)).is_err());
        assert!(S3::new(s("https://h/path", false)).is_err());
    }
}
