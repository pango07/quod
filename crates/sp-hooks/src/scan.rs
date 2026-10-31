//! After a keyed step: did a file it read or executed name a variable it
//! was not given? A program can read `$GITHUB_SHA` without the step's
//! script naming it; withheld, it saw nothing, and its result is not the
//! one GitHub would produce. Such a step runs again with the runner's whole
//! environment and is not recorded. `@actions/github` counts too: it reads
//! the event, the sha and the ref at import.
//!
//! Each blob is scanned once for a fixed pattern set and the answer is
//! memoized in the store (`<cas>/scan-v1/<hh>/<hash>`); a withheld name the
//! set does not cover (a token in a variable of another name) is scanned
//! for directly.

use std::collections::BTreeSet;
use std::path::PathBuf;

use aho_corasick::AhoCorasick;
use anyhow::Context;
use sp_cas::{Cas, Hash};

const MEMO: &str = "scan-v1";

/// Prefix families: a match on one stands for every name it starts.
const FAMILIES: &[&str] = &["GITHUB_RUN_", "ACTIONS_"];

/// The module that reads the workflow's context from the environment.
pub const ACTIONS_GITHUB: &str = "@actions/github";

fn patterns() -> Vec<String> {
    let mut p: Vec<String> = sp_key::VOLATILE.iter().map(|s| s.to_string()).collect();
    p.extend(FAMILIES.iter().map(|s| s.to_string()));
    p.extend(
        [
            "RUNNER_NAME",
            "RUNNER_TRACKING_ID",
            "GH_TOKEN",
            "INPUT_TOKEN",
            ACTIONS_GITHUB,
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    p.sort();
    p.dedup();
    p
}

fn memo_path(cas: &Cas, h: &Hash) -> PathBuf {
    let hex = h.hex();
    cas.dir().join(MEMO).join(&hex[..2]).join(hex)
}

/// The patterns `h`'s blob contains (memoized).
pub fn patterns_in(
    cas: &Cas,
    ac: &AhoCorasick,
    pats: &[String],
    h: &Hash,
) -> anyhow::Result<Vec<String>> {
    let memo = memo_path(cas, h);
    if let Ok(t) = std::fs::read_to_string(&memo) {
        return Ok(t.lines().map(String::from).collect());
    }
    let found = stream_matches(cas, ac, h)?;
    let mut names: Vec<String> = found.into_iter().map(|i| pats[i].clone()).collect();
    names.sort();
    std::fs::create_dir_all(memo.parent().unwrap())?;
    let tmp = memo.with_extension(format!("tmp.{}", ulid::Ulid::generate()));
    std::fs::write(&tmp, names.join("\n"))?;
    std::fs::rename(&tmp, &memo)?;
    Ok(names)
}

fn stream_matches(cas: &Cas, ac: &AhoCorasick, h: &Hash) -> anyhow::Result<BTreeSet<usize>> {
    use std::io::Read;
    let mut f = cas
        .open_blob(h)
        .with_context(|| format!("open blob {h} to scan"))?;
    // Chunks: a non-final chunk reports the matches that start before its
    // last OVERLAP bytes (patterns are shorter, so their right neighbour is
    // in the chunk); the next chunk starts with those bytes plus one byte of
    // left context, and reports from after that byte.
    const CHUNK: usize = 8 << 20;
    const OVERLAP: usize = 256;
    let mut found = BTreeSet::new();
    let mut buf: Vec<u8> = Vec::with_capacity(CHUNK + OVERLAP + 1);
    let mut from = 0usize;
    loop {
        let start = buf.len();
        buf.resize(start + CHUNK, 0);
        let mut filled = start;
        while filled < buf.len() {
            let n = f.read(&mut buf[filled..])?;
            if n == 0 {
                break;
            }
            filled += n;
        }
        let eof = filled < buf.len();
        buf.truncate(filled);
        let until = if eof { buf.len() } else { buf.len() - OVERLAP };
        found.extend(names_at_boundaries(&buf, ac, from, until));
        if eof {
            break;
        }
        buf = buf.split_off(buf.len() - OVERLAP - 1);
        from = 1;
    }
    Ok(found)
}

/// Pattern ids that occur in `bytes` as names (an identifier boundary on
/// both sides, except after a family prefix or around `@actions/github`),
/// for matches starting in `from..until`.
fn names_at_boundaries(
    bytes: &[u8],
    ac: &AhoCorasick,
    from: usize,
    until: usize,
) -> BTreeSet<usize> {
    let ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut out = BTreeSet::new();
    for m in ac.find_overlapping_iter(bytes) {
        if m.start() < from || m.start() >= until {
            continue;
        }
        let pat = &bytes[m.start()..m.end()];
        let module = pat.starts_with(b"@");
        let family = FAMILIES.iter().any(|f| f.as_bytes() == pat);
        let left_ok = module || m.start() == 0 || !ident(bytes[m.start() - 1]);
        let right_ok = module || family || m.end() == bytes.len() || !ident(bytes[m.end()]);
        if left_ok && right_ok {
            out.insert(m.pattern().as_usize());
        }
    }
    out
}

/// Which of `withheld` (and `@actions/github`) the files name, each with
/// the first file (by path) that names it.
pub fn referenced_by(
    cas: &Cas,
    files: &std::collections::BTreeMap<String, Hash>,
    withheld: &[String],
) -> anyhow::Result<Vec<(String, String)>> {
    let mut out: std::collections::BTreeMap<String, String> = Default::default();
    for (path, h) in files {
        for name in referenced(cas, &BTreeSet::from([*h]), withheld)? {
            out.entry(name).or_insert_with(|| path.clone());
        }
    }
    Ok(out.into_iter().collect())
}

/// Is this a file whose content can make a program read an environment
/// variable (code, a script, a config a tool interprets), as opposed to
/// pure data? Everything is, except documents and images, and the
/// repository's `.github/` automation (workflow definitions the runner
/// already evaluated, read by a step only as data, as `git status` does
/// when go build stamps VCS information).
pub fn may_drive_a_program(path: &str) -> bool {
    const DATA: &[&str] = &[
        ".md",
        ".markdown",
        ".rst",
        ".txt",
        ".png",
        ".jpg",
        ".jpeg",
        ".gif",
        ".svg",
        ".ico",
        ".pdf",
        ".webp",
    ];
    let lower = path.to_ascii_lowercase();
    !(lower.starts_with(".github/") || DATA.iter().any(|d| lower.ends_with(d)))
}

/// Which of `withheld` (and `@actions/github`) the blobs name.
pub fn referenced(
    cas: &Cas,
    blobs: &BTreeSet<Hash>,
    withheld: &[String],
) -> anyhow::Result<Vec<String>> {
    if withheld.is_empty() {
        return Ok(Vec::new());
    }
    let pats = patterns();
    let ac = AhoCorasick::builder()
        .match_kind(aho_corasick::MatchKind::Standard)
        .build(&pats)
        .context("scan patterns")?;
    let covered =
        |name: &str| pats.iter().any(|p| p == name) || FAMILIES.iter().any(|f| name.starts_with(f));
    let extra: Vec<&String> = withheld.iter().filter(|n| !covered(n)).collect();
    let extra_ac = (!extra.is_empty())
        .then(|| AhoCorasick::builder().build(&extra))
        .transpose()
        .context("scan names")?;
    let mut hits: BTreeSet<String> = BTreeSet::new();
    for h in blobs {
        for p in patterns_in(cas, &ac, &pats, h)? {
            if p == ACTIONS_GITHUB {
                hits.insert(p);
                continue;
            }
            for w in withheld {
                let family = FAMILIES.contains(&p.as_str()) && w.starts_with(&p);
                if *w == p || family {
                    hits.insert(w.clone());
                }
            }
        }
        if let Some(ac) = &extra_ac {
            for i in stream_matches(cas, ac, h)? {
                hits.insert(extra[i].clone());
            }
        }
    }
    Ok(hits.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_in_files_and_the_memo() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path()).unwrap();
        let a = cas
            .put(b"os.Getenv(\"GITHUB_SHA\") and GITHUB_REF_NAME and GITHUB_ACTIONS")
            .unwrap();
        let b = cas.put(b"package main; func main() {}").unwrap();
        let c = cas.put(b"require('@actions/github')").unwrap();
        let d = cas.put(b"echo $MY_TOKEN").unwrap();
        let w: Vec<String> = [
            "GITHUB_SHA",
            "GITHUB_REF_NAME",
            "GITHUB_RUN_ID",
            "MY_TOKEN",
            "GITHUB_ACTION",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let set = |hs: &[Hash]| hs.iter().copied().collect::<BTreeSet<_>>();
        // GITHUB_SHA from the memoized pattern set, GITHUB_REF_NAME (not in
        // the set) by a direct scan.
        assert_eq!(
            referenced(&cas, &set(&[a, b]), &w).unwrap(),
            ["GITHUB_REF_NAME", "GITHUB_SHA"]
        );
        let fam = cas.put(b"x GITHUB_RUN_ID y").unwrap();
        assert_eq!(
            referenced(&cas, &set(&[fam]), &w).unwrap(),
            ["GITHUB_RUN_ID"]
        );
        assert!(memo_path(&cas, &a).is_file(), "memoized");
        assert_eq!(
            referenced(&cas, &set(&[b]), &w).unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(referenced(&cas, &set(&[c]), &w).unwrap(), [ACTIONS_GITHUB]);
        assert_eq!(referenced(&cas, &set(&[d]), &w).unwrap(), ["MY_TOKEN"]);
        assert!(referenced(&cas, &set(&[a]), &[]).unwrap().is_empty());
        // Names, not substrings: GITHUB_ACTIONS is not GITHUB_ACTION, and a
        // name inside a longer identifier does not count.
        let e = cas
            .put(b"MY_GITHUB_SHA GITHUB_SHAX GITHUB_ACTIONS")
            .unwrap();
        assert!(referenced(&cas, &set(&[e]), &w).unwrap().is_empty());
        // A name split across the chunk boundary is still found.
        let mut big = vec![b' '; (8 << 20) - 5];
        big.extend_from_slice(b"GITHUB_SHA ");
        let f = cas.put(&big).unwrap();
        assert_eq!(referenced(&cas, &set(&[f]), &w).unwrap(), ["GITHUB_SHA"]);
    }
}
