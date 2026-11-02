//! The job container's root filesystem: an OCI image, unpacked once per
//! digest and shared read-only by every job that names it. The step key's
//! `image` field is the digest, so a different image is a different key.
//!
//! This is the seam for the rootfs module on exp/rootfs: `RootfsProvider`
//! is what the hooks need, `OciRootfs` is the temporary implementation
//! (registry API over curl, layers unpacked with tar, whiteouts applied).
//! Nothing above this module knows how the root was produced.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, anyhow, bail};
use quod_cas::Hash;
use serde::{Deserialize, Serialize};

/// An unpacked image.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Rootfs {
    /// The platform manifest digest (`sha256:...`): the image's identity.
    pub digest: String,
    /// The image as the workflow named it.
    pub reference: String,
    /// The unpacked root (read-only, shared; never written to).
    pub dir: PathBuf,
    /// The image config's environment (PATH and friends).
    pub env: Vec<(String, String)>,
    pub is_alpine: bool,
    /// How it was unpacked (`unpacker_id`): the same image unpacked another
    /// way (another tar, other whiteout rules) is another root.
    #[serde(default)]
    pub unpacker: String,
}

/// The unpack rules of this module: bump when what a layer becomes on
/// disk changes (whiteouts, permissions, ownership, what tar is told).
pub const UNPACKER_VERSION: &str = "oci-tar-1";

/// The unpacker's identity: `UNPACKER_VERSION` and the host tar that does
/// the extraction (its `--version` line), which decides how a layer's
/// entries land (sparse files, xattrs, links).
pub fn unpacker_id() -> String {
    static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ID.get_or_init(|| {
        let tar = Command::new("tar")
            .arg("--version")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|t| t.lines().next().map(str::to_string))
            .unwrap_or_else(|| "tar ?".into());
        format!("{UNPACKER_VERSION} {tar}")
    })
    .clone()
}

impl Rootfs {
    /// The step key's image pin: the image digest and how it was unpacked.
    pub fn key(&self) -> Hash {
        Hash::of(format!("{}\0{}", self.digest, self.unpacker).as_bytes())
    }
}

pub trait RootfsProvider {
    /// Resolves `image` (`name[:tag][@digest]`) and returns its unpacked root,
    /// fetching and unpacking it on first use.
    fn prepare(&self, image: &str) -> anyhow::Result<Rootfs>;
}

/// Images unpacked under `<root>/<digest hex>` from their registry.
pub struct OciRootfs {
    root: PathBuf,
}

/// A parsed image reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageRef {
    pub registry: String,
    pub repository: String,
    /// A tag or a `sha256:` digest.
    pub reference: String,
}

impl ImageRef {
    pub fn parse(image: &str) -> anyhow::Result<ImageRef> {
        let image = image.trim();
        if image.is_empty() {
            bail!("empty image reference");
        }
        // The last colon after the last slash is the tag separator.
        let split_tag = |image: &str| -> (usize, Option<usize>) {
            let slash = image.rfind('/').map_or(0, |i| i + 1);
            (slash, image[slash..].rfind(':').map(|i| slash + i))
        };
        let (name, reference) = if let Some((n, d)) = image.split_once('@') {
            // `name:tag@digest` (what `docker images --digests` prints and
            // Renovate pins): the digest names the image, the tag is only a
            // label for people and is dropped.
            let name = match split_tag(n) {
                (_, Some(i)) => &n[..i],
                (_, None) => n,
            };
            if !d.contains(':') {
                bail!("image reference {image:?}: a digest is `algorithm:hex`");
            }
            (name, d.to_string())
        } else {
            match split_tag(image) {
                (_, Some(i)) => (&image[..i], image[i + 1..].to_string()),
                (_, None) => (image, "latest".to_string()),
            }
        };
        if name.is_empty() {
            bail!("image reference {image:?} names no repository");
        }
        let mut parts: Vec<&str> = name.split('/').collect();
        let registry = if parts.len() > 1
            && (parts[0].contains('.') || parts[0].contains(':') || parts[0] == "localhost")
        {
            parts.remove(0).to_string()
        } else {
            "docker.io".to_string()
        };
        if registry == "docker.io" && parts.len() == 1 {
            parts.insert(0, "library");
        }
        Ok(ImageRef {
            registry,
            repository: parts.join("/"),
            reference,
        })
    }

    fn api_host(&self) -> &str {
        if self.registry == "docker.io" {
            "registry-1.docker.io"
        } else {
            &self.registry
        }
    }
}

const MANIFEST_ACCEPT: &str = "application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.v2+json";

#[derive(Deserialize)]
struct Descriptor {
    digest: String,
    #[serde(default)]
    platform: Option<Platform>,
}

#[derive(Deserialize)]
struct Platform {
    architecture: String,
    os: String,
    #[serde(default)]
    variant: Option<String>,
}

#[derive(Deserialize)]
struct ManifestDoc {
    #[serde(rename = "mediaType", default)]
    media_type: String,
    #[serde(default)]
    manifests: Vec<Descriptor>,
    #[serde(default)]
    config: Option<Descriptor>,
    #[serde(default)]
    layers: Vec<Descriptor>,
}

#[derive(Deserialize)]
struct ConfigDoc {
    #[serde(default)]
    config: ConfigInner,
}

#[derive(Deserialize, Default)]
struct ConfigInner {
    #[serde(rename = "Env", default)]
    env: Vec<String>,
}

/// What the `.complete` marker records, so a prepared root is reused
/// without a registry round trip.
#[derive(Serialize, Deserialize)]
struct Complete {
    digest: String,
    env: Vec<(String, String)>,
    is_alpine: bool,
}

impl OciRootfs {
    pub fn new(root: impl Into<PathBuf>) -> OciRootfs {
        OciRootfs { root: root.into() }
    }

    /// Where an image is unpacked: per digest and per unpacker, so a root
    /// another unpacker made is never taken for this one's.
    fn dir_for(&self, digest: &str) -> PathBuf {
        let tag = &Hash::of(unpacker_id().as_bytes()).hex()[..12];
        self.root
            .join(format!("{}-u{tag}", digest.replace(':', "-")))
    }

    /// The host's platform as a registry names it.
    fn platform() -> (&'static str, Option<&'static str>) {
        match std::env::consts::ARCH {
            "aarch64" => ("arm64", Some("v8")),
            "x86_64" => ("amd64", None),
            "arm" => ("arm", Some("v7")),
            other => (other, None),
        }
    }

    fn unpack(&self, r: &ImageRef, reference: &str) -> anyhow::Result<Rootfs> {
        let client = Registry::connect(r)?;
        // A tag is resolved on every job (it moves), but by a HEAD request
        // alone when what it names is unpacked already: no manifest is
        // downloaded (on Docker Hub a manifest GET counts against the pull
        // rate limit, a HEAD does not). The HEAD's Docker-Content-Digest is
        // the registry's word, unverified (no body comes with it), so it is
        // only a cache hint (`hinted`): it is taken only when that digest
        // names a root unpacked and verified before (the manifest's bytes
        // hashed to that digest, every blob to its own), directly or as the
        // platform manifest of an index resolved and verified before. Then
        // the job runs on exactly the image that digest names; which digest
        // the tag names now is the registry's word, as it is for a GET (the
        // tag-to-manifest binding is never signed). There is no other pull
        // to confirm it against: under the container hooks the runner pulls
        // nothing, this is the pull. Anything else takes the verified GET.
        if !reference.starts_with("sha256:")
            && let Some(d) = client.head_digest(reference)?
            && let Some(root) = self.hinted(r, &d)?
        {
            return Ok(root);
        }
        let (doc, digest) = client.manifest(reference)?;
        let index_digest = digest.clone();
        let (doc, digest) = if doc.media_type.ends_with("index.v1+json")
            || doc.media_type.ends_with("manifest.list.v2+json")
            || !doc.manifests.is_empty()
        {
            let (arch, variant) = Self::platform();
            let pick = doc
                .manifests
                .iter()
                .filter(|m| {
                    m.platform
                        .as_ref()
                        .is_some_and(|p| p.os == "linux" && p.architecture == arch)
                })
                .max_by_key(|m| {
                    let v = m.platform.as_ref().and_then(|p| p.variant.as_deref());
                    i32::from(v == variant) + i32::from(v.is_none())
                })
                .with_context(|| {
                    format!("{}: no linux/{arch} manifest in the index", r.repository)
                })?;
            let d = pick.digest.clone();
            let picked = client.manifest(&d)?;
            // Remembered for the next resolution of the same index (a tag
            // that has not moved): content-addressed, never stale.
            let alias = self.alias_for(&index_digest);
            if let Some(dir) = alias.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let tmp = alias.with_extension(format!("tmp.{}", ulid::Ulid::generate()));
            std::fs::write(&tmp, &picked.1)?;
            std::fs::rename(&tmp, &alias)?;
            picked
        } else {
            (doc, digest)
        };
        if doc.layers.is_empty() {
            bail!("{}@{digest}: manifest has no layers", r.repository);
        }
        let dest = self.dir_for(&digest);
        if let Some(done) = complete(&dest)? {
            // The manifest that names it was just verified (`manifest`), and
            // its layers were when it was unpacked.
            self.mark_verified(&digest)?;
            return Ok(Rootfs {
                digest,
                reference: format!("{}/{}@{}", r.registry, r.repository, done.digest),
                dir: dest,
                env: done.env,
                is_alpine: done.is_alpine,
                unpacker: unpacker_id(),
            });
        }
        let config: ConfigDoc = match &doc.config {
            Some(c) => serde_json::from_slice(&client.blob(&c.digest)?)?,
            None => ConfigDoc {
                config: ConfigInner::default(),
            },
        };
        let env: Vec<(String, String)> = config
            .config
            .env
            .iter()
            .filter_map(|kv| {
                kv.split_once('=')
                    .map(|(k, v)| (k.to_string(), v.to_string()))
            })
            .collect();

        std::fs::create_dir_all(&self.root)?;
        let tmp = self.root.join(format!("tmp.{}", ulid::Ulid::generate()));
        std::fs::create_dir_all(&tmp)?;
        let result = (|| -> anyhow::Result<()> {
            for (i, layer) in doc.layers.iter().enumerate() {
                let file = self
                    .root
                    .join(format!("layer.{}.{i}", ulid::Ulid::generate()));
                client.blob_to_file(&layer.digest, &file)?;
                let res = apply_layer(&file, &tmp);
                let _ = std::fs::remove_file(&file);
                res.with_context(|| format!("layer {} ({})", i, layer.digest))?;
            }
            Ok(())
        })();
        if let Err(e) = result {
            let _ = quod_cas::remove_any(&tmp);
            return Err(e);
        }
        let is_alpine = std::fs::read_to_string(tmp.join("etc/os-release"))
            .is_ok_and(|s| s.lines().any(|l| l.trim() == "ID=alpine"));
        let done = Complete {
            digest: digest.clone(),
            env: env.clone(),
            is_alpine,
        };
        std::fs::write(tmp.join(".complete"), serde_json::to_vec_pretty(&done)?)?;
        match std::fs::rename(&tmp, &dest) {
            Ok(()) => {}
            // Another process unpacked the same digest meanwhile.
            Err(_) if complete(&dest)?.is_some() => {
                let _ = quod_cas::remove_any(&tmp);
            }
            Err(e) => return Err(e).with_context(|| format!("publish {}", dest.display())),
        }
        self.mark_verified(&digest)?;
        Ok(Rootfs {
            reference: format!("{}/{}@{}", r.registry, r.repository, digest),
            digest,
            dir: dest,
            env,
            is_alpine,
            unpacker: unpacker_id(),
        })
    }
}

fn complete(dir: &Path) -> anyhow::Result<Option<Complete>> {
    match std::fs::read(dir.join(".complete")) {
        Ok(b) => Ok(Some(serde_json::from_slice(&b)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

impl OciRootfs {
    /// Where an index digest's platform manifest digest is remembered. An
    /// index is content-addressed, so for one platform the answer never
    /// changes; remembering it saves the registry round trips (about two
    /// seconds) on every job that names an image by index digest.
    /// Only ever written after the index and the platform manifest were
    /// fetched and verified (`Registry::manifest`); `v2` leaves behind the
    /// aliases written before manifests were verified.
    fn alias_for(&self, digest: &str) -> PathBuf {
        let (arch, _) = Self::platform();
        self.root
            .join(format!("index-v2-{arch}-{}", digest.replace(':', "-")))
    }

    /// Next to an unpacked root (outside it, so no step sees it): the root
    /// was unpacked from, or reached through, a manifest whose bytes were
    /// verified against its digest. Roots unpacked before manifests were
    /// verified have none, so no cache hint is taken for them until a
    /// verified GET names them again.
    fn verified_marker(&self, digest: &str) -> PathBuf {
        let dir = self.dir_for(digest);
        let mut name = dir
            .file_name()
            .expect("dir_for names a file")
            .to_os_string();
        name.push(".verified");
        dir.with_file_name(name)
    }

    fn mark_verified(&self, digest: &str) -> anyhow::Result<()> {
        let marker = self.verified_marker(digest);
        if !marker.exists() {
            let tmp = marker.with_extension(format!("tmp.{}", ulid::Ulid::generate()));
            std::fs::write(&tmp, digest)?;
            std::fs::rename(&tmp, &marker)?;
        }
        Ok(())
    }

    /// The root a digest names, if it is unpacked and was verified: the
    /// digest itself, or the platform manifest an index digest was resolved
    /// to. For a digest from an unverified source (a HEAD's header, the
    /// workflow's reference) nothing is fetched.
    fn hinted(&self, r: &ImageRef, digest: &str) -> anyhow::Result<Option<Rootfs>> {
        let verified = |d: &str| -> anyhow::Result<Option<Rootfs>> {
            if !self.verified_marker(d).is_file() {
                return Ok(None);
            }
            self.unpacked(r, d)
        };
        if let Some(root) = verified(digest)? {
            return Ok(Some(root));
        }
        match std::fs::read_to_string(self.alias_for(digest)) {
            Ok(platform) => verified(platform.trim()),
            Err(_) => Ok(None),
        }
    }

    fn unpacked(&self, r: &ImageRef, digest: &str) -> anyhow::Result<Option<Rootfs>> {
        Ok(complete(&self.dir_for(digest))?.map(|done| Rootfs {
            dir: self.dir_for(digest),
            digest: digest.to_string(),
            reference: format!("{}/{}@{}", r.registry, r.repository, done.digest),
            env: done.env,
            is_alpine: done.is_alpine,
            unpacker: unpacker_id(),
        }))
    }
}

impl RootfsProvider for OciRootfs {
    fn prepare(&self, image: &str) -> anyhow::Result<Rootfs> {
        let r = ImageRef::parse(image)?;
        // A digest reference may already be unpacked and verified, directly
        // or as the platform manifest of an index: no network at all.
        if r.reference.starts_with("sha256:")
            && let Some(root) = self.hinted(&r, &r.reference)?
        {
            return Ok(root);
        }
        let root = self.unpack(&r, &r.reference)?;
        if r.reference.starts_with("sha256:") && root.digest != r.reference {
            let alias = self.alias_for(&r.reference);
            let tmp = alias.with_extension(format!("tmp.{}", ulid::Ulid::generate()));
            std::fs::write(&tmp, &root.digest)?;
            std::fs::rename(&tmp, &alias)?;
        }
        Ok(root)
    }
}

/// Applies one layer tarball to `root`: whiteouts first (`.wh.<name>`
/// removes name, `.wh..wh..opq` empties the directory), then everything
/// else extracted in place with ownership and modes preserved.
fn apply_layer(tar: &Path, root: &Path) -> anyhow::Result<()> {
    let list = Command::new("tar")
        .arg("-tf")
        .arg(tar)
        .stderr(Stdio::inherit())
        .output()
        .context("tar -t")?;
    if !list.status.success() {
        bail!("tar -t failed on {}", tar.display());
    }
    let mut whiteouts = 0;
    for name in String::from_utf8_lossy(&list.stdout).lines() {
        let name = name.trim_end_matches('/');
        let (dir, base) = match name.rsplit_once('/') {
            Some((d, b)) => (d, b),
            None => ("", name),
        };
        if base == ".wh..wh..opq" {
            let p = safe_join(root, dir)?;
            if p.is_dir() {
                for e in std::fs::read_dir(&p)? {
                    quod_cas::remove_any(&e?.path())?;
                }
            }
            whiteouts += 1;
        } else if let Some(victim) = base.strip_prefix(".wh.") {
            let p = safe_join(root, &format!("{dir}/{victim}"))?;
            quod_cas::remove_any(&p)?;
            whiteouts += 1;
        }
    }
    let st = Command::new("tar")
        .args(["-xf"])
        .arg(tar)
        .args(["-C"])
        .arg(root)
        .args([
            "--exclude=.wh.*",
            "--exclude=*/.wh.*",
            "--no-same-owner",
            "--numeric-owner",
            "--same-permissions",
            "--delay-directory-restore",
        ])
        .stderr(Stdio::inherit())
        .status()
        .context("tar -x")?;
    if !st.success() {
        bail!("tar -x failed on {}", tar.display());
    }
    tracing::info!(target: "quod", layer = %tar.display(), whiteouts, "layer applied");
    Ok(())
}

/// `root/rel` with `..` refused: layer entries are untrusted.
fn safe_join(root: &Path, rel: &str) -> anyhow::Result<PathBuf> {
    let rel = rel.trim_start_matches('/');
    if rel.split('/').any(|c| c == "..") {
        bail!("layer entry escapes the root: {rel:?}");
    }
    Ok(root.join(rel))
}

/// A registry client over curl: anonymous bearer tokens, manifests, blobs.
struct Registry {
    base: String,
    token: Option<String>,
}

impl Registry {
    fn connect(r: &ImageRef) -> anyhow::Result<Registry> {
        let base = format!("https://{}/v2/{}", r.api_host(), r.repository);
        let mut reg = Registry { base, token: None };
        // Probe; a 401 tells us where to get a token.
        let (status, headers, _) = reg.get(
            &format!("{}/manifests/{}", reg.base, r.reference),
            MANIFEST_ACCEPT,
            true,
        )?;
        if status == 401 {
            let challenge = headers
                .iter()
                .find_map(|h| h.strip_prefix("www-authenticate: "))
                .context("401 without WWW-Authenticate")?
                .trim()
                .to_string();
            reg.token = Some(fetch_token(&challenge)?);
        }
        Ok(reg)
    }

    /// GET with curl: (status, lowercase header lines, body).
    fn get(
        &self,
        url: &str,
        accept: &str,
        head: bool,
    ) -> anyhow::Result<(u16, Vec<String>, Vec<u8>)> {
        let hdr = std::env::temp_dir().join(format!("quod-hdr-{}", ulid::Ulid::generate()));
        let mut c = Command::new("curl");
        c.args(["-sS", "-L", "--max-time", "600", "-D"])
            .arg(&hdr)
            .args(["-H", &format!("Accept: {accept}")]);
        if let Some(t) = &self.token {
            c.args(["-H", &format!("Authorization: Bearer {t}")]);
        }
        if head {
            c.arg("-I");
        }
        c.arg(url).stderr(Stdio::inherit());
        let out = c.output().context("curl")?;
        let headers = std::fs::read_to_string(&hdr).unwrap_or_default();
        let _ = std::fs::remove_file(&hdr);
        // The last response's headers (after redirects).
        let last = headers
            .split("\r\n\r\n")
            .filter(|b| !b.trim().is_empty())
            .last()
            .unwrap_or("");
        let mut lines: Vec<String> = last.lines().map(|l| l.to_ascii_lowercase()).collect();
        let status = lines
            .first()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        lines.retain(|l| l.contains(':'));
        if !out.status.success() && status == 0 {
            bail!(
                "curl {url} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Ok((status, lines, out.stdout))
    }

    /// The digest a reference names, by HEAD (no body): `None` when the
    /// registry does not say (no Docker-Content-Digest, or not 200).
    fn head_digest(&self, reference: &str) -> anyhow::Result<Option<String>> {
        let url = format!("{}/manifests/{reference}", self.base);
        let (status, headers, _) = self.get(&url, MANIFEST_ACCEPT, true)?;
        if status != 200 {
            return Ok(None);
        }
        Ok(headers
            .iter()
            .find_map(|h| h.strip_prefix("docker-content-digest:"))
            .map(|d| d.trim().to_string())
            .filter(|d| {
                d.strip_prefix("sha256:")
                    .is_some_and(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
            }))
    }

    fn manifest(&self, reference: &str) -> anyhow::Result<(ManifestDoc, String)> {
        let url = format!("{}/manifests/{reference}", self.base);
        let (status, headers, body) = self.get(&url, MANIFEST_ACCEPT, false)?;
        if status != 200 {
            bail!(
                "{url}: HTTP {status}: {}",
                String::from_utf8_lossy(&body)
                    .chars()
                    .take(300)
                    .collect::<String>()
            );
        }
        let header = headers
            .iter()
            .find_map(|h| h.strip_prefix("docker-content-digest:"))
            .map(str::trim);
        let digest = manifest_digest(&url, reference, header, &body)?;
        let doc: ManifestDoc =
            serde_json::from_slice(&body).with_context(|| format!("{url}: not a manifest"))?;
        Ok((doc, digest))
    }

    fn blob(&self, digest: &str) -> anyhow::Result<Vec<u8>> {
        let url = format!("{}/blobs/{digest}", self.base);
        let (status, _, body) = self.get(&url, "*/*", false)?;
        if status != 200 {
            bail!("{url}: HTTP {status}");
        }
        verify_sha256(digest, &body)?;
        Ok(body)
    }

    fn blob_to_file(&self, digest: &str, dest: &Path) -> anyhow::Result<()> {
        let url = format!("{}/blobs/{digest}", self.base);
        let mut c = Command::new("curl");
        c.args(["-fsS", "-L", "--max-time", "1800", "-o"]).arg(dest);
        if let Some(t) = &self.token {
            c.args(["-H", &format!("Authorization: Bearer {t}")]);
        }
        let st = c
            .arg(&url)
            .stderr(Stdio::inherit())
            .status()
            .context("curl")?;
        if !st.success() {
            bail!("downloading {url} failed");
        }
        let mut f = std::fs::File::open(dest)?;
        let mut h = Sha256::new();
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
        }
        let got = format!("sha256:{}", h.finish_hex());
        if got != digest {
            let _ = std::fs::remove_file(dest);
            bail!("{url}: content digest {got} does not match");
        }
        Ok(())
    }
}

/// `Bearer realm="...",service="...",scope="..."` -> a token (anonymous).
fn fetch_token(challenge: &str) -> anyhow::Result<String> {
    let rest = challenge
        .strip_prefix("bearer ")
        .ok_or_else(|| anyhow!("unsupported auth challenge {challenge:?}"))?;
    let mut params: BTreeMap<String, String> = BTreeMap::new();
    for kv in rest.split(',') {
        if let Some((k, v)) = kv.trim().split_once('=') {
            params.insert(k.trim().to_string(), v.trim().trim_matches('"').to_string());
        }
    }
    let realm = params.get("realm").context("challenge without realm")?;
    let mut url = format!("{realm}?");
    for k in ["service", "scope"] {
        if let Some(v) = params.get(k) {
            url += &format!("{k}={v}&");
        }
    }
    let out = Command::new("curl")
        .args(["-fsS", "--max-time", "60"])
        .arg(&url)
        .stderr(Stdio::inherit())
        .output()
        .context("curl token")?;
    if !out.status.success() {
        bail!("token request to {url} failed");
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    v.get("token")
        .or_else(|| v.get("access_token"))
        .and_then(|t| t.as_str())
        .map(String::from)
        .context("token response without a token")
}

/// A manifest's digest is the hash of its bytes, never the registry's
/// word for it: a Docker-Content-Digest that differs, or a digest reference
/// the bytes do not hash to, is refused.
fn manifest_digest(
    url: &str,
    reference: &str,
    header: Option<&str>,
    body: &[u8],
) -> anyhow::Result<String> {
    let digest = format!("sha256:{}", sha256_hex(body));
    if let Some(h) = header.filter(|h| h.starts_with("sha256:"))
        && h != digest
    {
        bail!("{url}: Docker-Content-Digest {h} is not the manifest's digest ({digest})");
    }
    if reference.starts_with("sha256:") && digest != reference {
        bail!("{url}: digest mismatch ({digest})");
    }
    Ok(digest)
}

fn verify_sha256(digest: &str, body: &[u8]) -> anyhow::Result<()> {
    let got = format!("sha256:{}", sha256_hex(body));
    if got != digest {
        bail!("blob {digest}: content hashes to {got}");
    }
    Ok(())
}

fn sha256_hex(b: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(b);
    h.finish_hex()
}

/// SHA-256 (FIPS 180-4), enough to verify registry content digests without
/// another dependency.
struct Sha256 {
    state: [u32; 8],
    buf: Vec<u8>,
    len: u64,
}

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

impl Sha256 {
    fn new() -> Sha256 {
        Sha256 {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            buf: Vec::with_capacity(64),
            len: 0,
        }
    }

    fn update(&mut self, mut data: &[u8]) {
        self.len += data.len() as u64;
        if !self.buf.is_empty() {
            let need = 64 - self.buf.len();
            let take = need.min(data.len());
            self.buf.extend_from_slice(&data[..take]);
            data = &data[take..];
            if self.buf.len() == 64 {
                let block: [u8; 64] = self.buf[..].try_into().unwrap();
                self.block(&block);
                self.buf.clear();
            }
        }
        while data.len() >= 64 {
            let block: [u8; 64] = data[..64].try_into().unwrap();
            self.block(&block);
            data = &data[64..];
        }
        self.buf.extend_from_slice(data);
    }

    fn block(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes(block[4 * i..4 * i + 4].try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (s, v) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *s = s.wrapping_add(v);
        }
    }

    fn finish_hex(mut self) -> String {
        let bits = self.len.wrapping_mul(8);
        let mut pad = vec![0x80u8];
        while (self.buf.len() + pad.len()) % 64 != 56 {
            pad.push(0);
        }
        pad.extend_from_slice(&bits.to_be_bytes());
        let len_before = self.len;
        self.update(&pad);
        self.len = len_before;
        let mut s = String::with_capacity(64);
        for v in self.state {
            s += &format!("{v:08x}");
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_image_key_pins_the_unpacker() {
        let r = |unpacker: &str| Rootfs {
            digest: "sha256:abc".into(),
            reference: "golang@sha256:abc".into(),
            dir: PathBuf::from("/x"),
            env: vec![],
            is_alpine: false,
            unpacker: unpacker.into(),
        };
        assert_eq!(
            r("oci-tar-1 tar (GNU tar) 1.34").key(),
            r("oci-tar-1 tar (GNU tar) 1.34").key()
        );
        assert_ne!(
            r("oci-tar-1 tar (GNU tar) 1.34").key(),
            r("oci-tar-1 tar (GNU tar) 1.35").key()
        );
        assert_ne!(r("oci-tar-1 x").key(), r("oci-tar-2 x").key());
        assert!(unpacker_id().starts_with(UNPACKER_VERSION));
        let dir = OciRootfs::new("/state/rootfs").dir_for("sha256:abc");
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            name.starts_with("sha256-abc-u") && name.len() == "sha256-abc-u".len() + 12,
            "{name}"
        );
    }

    #[test]
    fn image_refs() {
        let r = ImageRef::parse("golang:1.27.3").unwrap();
        assert_eq!(
            r,
            ImageRef {
                registry: "docker.io".into(),
                repository: "library/golang".into(),
                reference: "1.27.3".into()
            }
        );
        let r = ImageRef::parse("golang@sha256:abc").unwrap();
        assert_eq!(r.reference, "sha256:abc");
        assert_eq!(ImageRef::parse("node").unwrap().reference, "latest");
        let r = ImageRef::parse("ghcr.io/owner/img:v1").unwrap();
        assert_eq!(
            (
                r.registry.as_str(),
                r.repository.as_str(),
                r.reference.as_str()
            ),
            ("ghcr.io", "owner/img", "v1")
        );
        let r = ImageRef::parse("localhost:5000/x/y").unwrap();
        assert_eq!(
            (r.registry.as_str(), r.repository.as_str()),
            ("localhost:5000", "x/y")
        );
        assert!(ImageRef::parse(" ").is_err());
        // name:tag@digest: the digest names the image, the tag is dropped.
        let r = ImageRef::parse("python:3.12-slim@sha256:abc").unwrap();
        assert_eq!(
            r,
            ImageRef {
                registry: "docker.io".into(),
                repository: "library/python".into(),
                reference: "sha256:abc".into()
            }
        );
        let r = ImageRef::parse("localhost:5000/x/y:v2@sha256:def").unwrap();
        assert_eq!(
            (
                r.registry.as_str(),
                r.repository.as_str(),
                r.reference.as_str()
            ),
            ("localhost:5000", "x/y", "sha256:def")
        );
        let r = ImageRef::parse("localhost:5000/x/y@sha256:def").unwrap();
        assert_eq!(r.repository, "x/y");
        assert!(ImageRef::parse("python:3.12@abc").is_err());
        assert!(ImageRef::parse(":3.12").is_err());
    }

    /// N3 (Fable's review of 75ebbea): a manifest's digest is its bytes'
    /// hash; the registry's header is checked against it, never trusted.
    #[test]
    fn manifest_digests_are_verified() {
        let body = b"{}";
        let d = format!("sha256:{}", sha256_hex(body));
        assert_eq!(manifest_digest("u", "latest", None, body).unwrap(), d);
        assert_eq!(manifest_digest("u", "latest", Some(&d), body).unwrap(), d);
        assert_eq!(manifest_digest("u", &d, Some(&d), body).unwrap(), d);
        let other = format!("sha256:{}", sha256_hex(b"other"));
        assert!(manifest_digest("u", "latest", Some(&other), body).is_err());
        assert!(manifest_digest("u", &other, None, body).is_err());
    }

    /// N3: the HEAD's digest (or a digest reference) is a cache hint only:
    /// taken for a root unpacked and verified before, directly or through
    /// an alias written after verified GETs; a root without the verified
    /// marker (unpacked before manifests were verified) and an alias of the
    /// old naming are not. Before, any unpacked root was taken.
    #[test]
    fn a_head_digest_is_only_a_hint_for_verified_roots() {
        let tmp = tempfile::tempdir().unwrap();
        let o = OciRootfs::new(tmp.path());
        let r = ImageRef::parse("python:3.12-slim").unwrap();
        let (index, platform) = ("sha256:1111", "sha256:2222");
        let unpack = |d: &str| {
            let dir = o.dir_for(d);
            std::fs::create_dir_all(&dir).unwrap();
            let done = Complete {
                digest: d.into(),
                env: vec![("GPG_KEY".into(), "x".into())],
                is_alpine: false,
            };
            std::fs::write(dir.join(".complete"), serde_json::to_vec(&done).unwrap()).unwrap();
        };
        unpack(platform);
        let (arch, _) = OciRootfs::platform();
        // The alias the old code wrote: not taken.
        std::fs::write(
            tmp.path().join(format!("index-{arch}-sha256-1111")),
            platform,
        )
        .unwrap();
        assert!(o.hinted(&r, index).unwrap().is_none());
        // Unpacked but never verified: not taken, directly or by alias.
        std::fs::write(o.alias_for(index), platform).unwrap();
        assert!(o.hinted(&r, platform).unwrap().is_none());
        assert!(o.hinted(&r, index).unwrap().is_none());
        // Verified: taken both ways, and the marker is not in the root.
        o.mark_verified(platform).unwrap();
        let root = o.hinted(&r, platform).unwrap().unwrap();
        assert_eq!(root.digest, platform);
        assert!(!root.dir.join(".verified").exists());
        assert_eq!(o.hinted(&r, index).unwrap().unwrap().digest, platform);
        // A digest nothing is unpacked for.
        assert!(o.hinted(&r, "sha256:3333").unwrap().is_none());
    }

    #[test]
    fn sha256_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let long = vec![b'a'; 1_000_000];
        assert_eq!(
            sha256_hex(&long),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
        // Streaming in odd chunks agrees with one shot.
        let mut h = Sha256::new();
        for chunk in long.chunks(777) {
            h.update(chunk);
        }
        assert_eq!(h.finish_hex(), sha256_hex(&long));
    }

    #[test]
    fn whiteouts_are_applied() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("root");
        std::fs::create_dir_all(root.join("etc/keep")).unwrap();
        std::fs::write(root.join("etc/old"), "old").unwrap();
        std::fs::write(root.join("etc/keep/x"), "x").unwrap();
        std::fs::create_dir_all(root.join("opt/gone")).unwrap();
        std::fs::write(root.join("opt/gone/f"), "f").unwrap();
        // Layer: whiteout etc/old, make opt opaque with a new file, add etc/new.
        let stage = tmp.path().join("stage");
        std::fs::create_dir_all(stage.join("etc")).unwrap();
        std::fs::create_dir_all(stage.join("opt")).unwrap();
        std::fs::write(stage.join("etc/.wh.old"), "").unwrap();
        std::fs::write(stage.join("etc/new"), "new").unwrap();
        std::fs::write(stage.join("opt/.wh..wh..opq"), "").unwrap();
        std::fs::write(stage.join("opt/fresh"), "fresh").unwrap();
        let tar = tmp.path().join("layer.tar");
        let st = Command::new("tar")
            .arg("-cf")
            .arg(&tar)
            .arg("-C")
            .arg(&stage)
            .args(["etc", "opt"])
            .status()
            .unwrap();
        assert!(st.success());
        apply_layer(&tar, &root).unwrap();
        assert!(!root.join("etc/old").exists());
        assert!(!root.join("etc/.wh.old").exists());
        assert_eq!(
            std::fs::read_to_string(root.join("etc/new")).unwrap(),
            "new"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("etc/keep/x")).unwrap(),
            "x"
        );
        assert!(!root.join("opt/gone").exists());
        assert!(!root.join("opt/.wh..wh..opq").exists());
        assert_eq!(
            std::fs::read_to_string(root.join("opt/fresh")).unwrap(),
            "fresh"
        );
    }

    #[test]
    fn layer_entries_cannot_escape() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(safe_join(tmp.path(), "a/../../b").is_err());
        assert_eq!(
            safe_join(tmp.path(), "/a/b").unwrap(),
            tmp.path().join("a/b")
        );
    }
}
