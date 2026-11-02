//! After a keyed step: did a file it read or executed name a withheld
//! variable whose value is the same from run to run, so that a program
//! could have made the step's result depend on it? Such a step runs again
//! with the runner's whole environment and is not recorded.
//! `@actions/github` counts too: it reads the event, the actor and the job
//! at import.
//!
//! Not every withheld name is scanned for (`scanned`). The scan is not a
//! secrecy control: a keyed step never gets a withheld value, only the
//! placeholder `surface::WITHHELD`, so nothing it records can hold or be
//! derived from the real one; and the placeholder is fixed and the key
//! names every withheld name, so a replay is exactly what a fresh keyed run
//! gives. What the scan is for is fidelity to GitHub: a result computed
//! from the placeholder where GitHub would have used the real value. For a
//! value that is new on every run or commit (the sha, the run id, a token)
//! that question is answered by the value itself, not by its name: a
//! result that holds the placeholder or its 7-byte prefix (the short sha)
//! is caught (`surface::PLACEHOLDER_MARKS`), a use that fails (an unknown
//! object, a rejected token) or reaches for the network runs again with
//! the runner's environment, and a presence check sees a set variable as on
//! GitHub. Libraries name those variables for telemetry and metadata
//! everywhere (langsmith's runtime env, vitest's CLI, codecov), and
//! scanning for their names made whole test stacks unrecordable. Names
//! whose value is stable for a branch or a step (the actor, the job, the
//! action, the event payload's path) are still scanned for: a result
//! derived from one of those without holding it would be a stable result
//! GitHub would not give. DECISIONS has the argument.
//!
//! Each blob is scanned once for a fixed pattern set and the answer is
//! memoized in the store (`<cas>/scan-<set>/<hh>/<hash>`, `<set>` a hash
//! of the pattern set, so a changed set never reads an old answer).

use std::collections::BTreeSet;
use std::path::PathBuf;

use aho_corasick::AhoCorasick;
use anyhow::Context;
use quod_cas::{Cas, Hash};

/// The module that reads the workflow's context from the environment.
pub const ACTIONS_GITHUB: &str = "@actions/github";

/// Withheld names whose value is new on every run (or commit), or is a
/// credential: not scanned for (see the module docs). `GITHUB_RUN_`,
/// `RUNNER_` and `ACTIONS_` are families.
const RUN_UNIQUE: &[&str] = &[
    "GITHUB_SHA",
    "GITHUB_WORKFLOW_SHA",
    "GITHUB_TOKEN",
    "INVOCATION_ID",
    "QUOD_RUN_ID",
];
const RUN_UNIQUE_FAMILIES: &[&str] = &["GITHUB_RUN_", "RUNNER_", "ACTIONS_"];

/// Is a withheld `name` (with its job's `value`) one a file naming makes
/// the step run again? Every withheld name is, unless its value is unique
/// to the run (`RUN_UNIQUE`) or holds a GitHub token: an unknown name is
/// scanned for.
pub fn scanned(name: &str, value: &str) -> bool {
    !(RUN_UNIQUE.contains(&name)
        || RUN_UNIQUE_FAMILIES.iter().any(|f| name.starts_with(f))
        || crate::step::holds_github_token(value))
}

/// The pattern set: the volatile names that are scanned for, and
/// `@actions/github`.
fn patterns() -> Vec<String> {
    let mut p: Vec<String> = quod_key::VOLATILE
        .iter()
        .filter(|n| scanned(n, ""))
        .map(|s| s.to_string())
        .collect();
    p.push(ACTIONS_GITHUB.to_string());
    p.sort();
    p.dedup();
    p
}

/// The memo's directory: named after the pattern set (and the matcher's
/// rules, `v2`), so answers computed for another set are never reused.
fn memo_dir(cas: &Cas) -> PathBuf {
    let set = Hash::of(format!("quod scan v3\0{}", patterns().join("\0")).as_bytes());
    cas.dir().join(format!("scan-{}", &set.hex()[..16]))
}

fn memo_path(cas: &Cas, h: &Hash) -> PathBuf {
    let hex = h.hex();
    memo_dir(cas).join(&hex[..2]).join(hex)
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
/// both sides, except around `@actions/github`), for matches starting in
/// `from..until`.
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
        let left_ok = module || m.start() == 0 || !ident(bytes[m.start() - 1]);
        let right_ok = module || m.end() == bytes.len() || !ident(bytes[m.end()]);
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
/// repository's workflow definitions (`.github/workflows/*.yml`, which the
/// runner already evaluated; a step reads them only as data, as `git
/// status` does when go build stamps VCS information). The rest of
/// `.github/` is scanned like any code: a local action
/// (`./.github/actions/x/dist/index.js`) runs and can read `$GITHUB_SHA`.
/// Text files are scanned too (a `.txt` can be a script's input).
pub fn may_drive_a_program(path: &str) -> bool {
    const DATA: &[&str] = &[
        ".md",
        ".markdown",
        ".rst",
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
    let workflow = lower
        .strip_prefix(".github/workflows/")
        .is_some_and(|f| !f.contains('/') && (f.ends_with(".yml") || f.ends_with(".yaml")));
    !(workflow || DATA.iter().any(|d| lower.ends_with(d)))
}

/// Which of `withheld` (and `@actions/github`) the blobs name. `withheld`
/// is the step's scanned names (`scanned`); with none, nothing is read.
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
    // A scanned name outside the pattern set (a withheld name Quod does not
    // know as volatile) is looked for directly.
    let extra: Vec<&String> = withheld.iter().filter(|n| !pats.contains(n)).collect();
    let extra_ac = (!extra.is_empty())
        .then(|| AhoCorasick::builder().build(&extra))
        .transpose()
        .context("scan names")?;
    let mut hits: BTreeSet<String> = BTreeSet::new();
    for h in blobs {
        for p in patterns_in(cas, &ac, &pats, h)? {
            if p == ACTIONS_GITHUB || withheld.contains(&p) {
                hits.insert(p);
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
    fn what_is_scanned() {
        // Code anywhere, .github/ included: a local action runs.
        for scanned in [
            ".github/actions/x/dist/index.js",
            ".github/workflows/scripts/bump-go.sh",
            ".github/workflows/sub/inner.yml",
            "notes.txt",
            "cmd/main.go",
            "go.mod",
        ] {
            assert!(may_drive_a_program(scanned), "{scanned}");
        }
        // Only the workflow definitions and documents or images are data.
        for data in [
            ".github/workflows/go.yml",
            ".github/workflows/Release.YAML",
            "README.md",
            "docs/logo.png",
        ] {
            assert!(!may_drive_a_program(data), "{data}");
        }
        // The memo is named after the pattern set.
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path()).unwrap();
        let d = memo_dir(&cas);
        assert!(
            d.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("scan-")
        );
        assert_ne!(d.file_name().unwrap(), "scan-v1");
    }

    #[test]
    fn names_in_files_and_the_memo() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path()).unwrap();
        let a = cas
            .put(b"os.Getenv(\"GITHUB_ACTOR\") and GITHUB_REF_NAME and GITHUB_ACTIONS")
            .unwrap();
        let b = cas.put(b"package main; func main() {}").unwrap();
        let c = cas.put(b"require('@actions/github')").unwrap();
        let d = cas.put(b"echo $MY_SETTING").unwrap();
        let w: Vec<String> = [
            "GITHUB_ACTOR",
            "GITHUB_REF_NAME",
            "MY_SETTING",
            "GITHUB_ACTION",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let set = |hs: &[Hash]| hs.iter().copied().collect::<BTreeSet<_>>();
        // GITHUB_ACTOR from the memoized pattern set, GITHUB_REF_NAME (not
        // in the set) by a direct scan.
        assert_eq!(
            referenced(&cas, &set(&[a, b]), &w).unwrap(),
            ["GITHUB_ACTOR", "GITHUB_REF_NAME"]
        );
        assert!(memo_path(&cas, &a).is_file(), "memoized");
        assert_eq!(
            referenced(&cas, &set(&[b]), &w).unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(referenced(&cas, &set(&[c]), &w).unwrap(), [ACTIONS_GITHUB]);
        assert_eq!(referenced(&cas, &set(&[d]), &w).unwrap(), ["MY_SETTING"]);
        assert!(referenced(&cas, &set(&[a]), &[]).unwrap().is_empty());
        // Only the names asked for: a file naming GITHUB_JOB is no hit for
        // a step that was not withheld it.
        let j = cas.put(b"process.env.GITHUB_JOB").unwrap();
        assert!(referenced(&cas, &set(&[j]), &w).unwrap().is_empty());
        // Names, not substrings: GITHUB_ACTIONS is not GITHUB_ACTION, and a
        // name inside a longer identifier does not count.
        let e = cas
            .put(b"MY_GITHUB_ACTOR GITHUB_ACTORX GITHUB_ACTIONS")
            .unwrap();
        assert!(referenced(&cas, &set(&[e]), &w).unwrap().is_empty());
        // A name split across the chunk boundary is still found.
        let mut big = vec![b' '; (8 << 20) - 5];
        big.extend_from_slice(b"GITHUB_ACTOR ");
        let f = cas.put(&big).unwrap();
        assert_eq!(referenced(&cas, &set(&[f]), &w).unwrap(), ["GITHUB_ACTOR"]);
    }

    /// Which withheld names a file may name without the step running
    /// again: those whose value is new every run or commit, or a token.
    /// The langsmith, vitest and jest (ci-info) files that made whole test
    /// stacks unrecordable name GITHUB_SHA and GITHUB_EVENT_NAME (keyed now,
    /// never withheld).
    #[test]
    fn run_unique_names_are_not_scanned_for() {
        for n in [
            "GITHUB_SHA",
            "GITHUB_WORKFLOW_SHA",
            "GITHUB_TOKEN",
            "GITHUB_RUN_ID",
            "GITHUB_RUN_NUMBER",
            "GITHUB_RUN_ATTEMPT",
            "RUNNER_NAME",
            "RUNNER_TRACKING_ID",
            "ACTIONS_RUNTIME_TOKEN",
            "INVOCATION_ID",
        ] {
            assert!(!scanned(n, "x"), "{n}");
        }
        assert!(!scanned(
            "GH_TOKEN",
            "ghs_0123456789abcdefghijABCDEFGHIJ0123"
        ));
        for n in [
            "GITHUB_ACTOR",
            "GITHUB_ACTOR_ID",
            "GITHUB_TRIGGERING_ACTOR",
            "GITHUB_JOB",
            "GITHUB_EVENT_PATH",
            "GITHUB_WORKFLOW_REF",
            "GITHUB_ACTION",
            "GITHUB_ACTION_REF",
            "GITHUB_ACTION_REPOSITORY",
            "GITHUB_ACTION_PATH",
            "GITHUB_RETENTION_DAYS",
            "SOMETHING_NEW",
        ] {
            assert!(scanned(n, "x"), "{n}");
        }
        assert!(!quod_key::is_volatile("GITHUB_EVENT_NAME"));
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path()).unwrap();
        let langsmith = cas
            .put(b"common_release_envs = [\n    \"VERCEL_GIT_COMMIT_SHA\",\n    \"GITHUB_SHA\",\n]\nfor env in common_release_envs:\n    os.environ.get(env)\n")
            .unwrap();
        let vitest = cas
            .put(b"const sha = process.env.GITHUB_SHA || process.env.CI_COMMIT_SHA;")
            .unwrap();
        let ci_info = cas
            .put(br#"[{"name":"GitHub Actions","constant":"GITHUB_ACTIONS","env":"GITHUB_ACTIONS","pr":{"GITHUB_EVENT_NAME":"pull_request"}}]"#)
            .unwrap();
        let blobs = [langsmith, vitest, ci_info].into_iter().collect();
        // What a step in a job with these withheld is scanned for.
        let job: Vec<(&str, &str)> = vec![
            ("GITHUB_SHA", "0123"),
            ("GITHUB_RUN_ID", "1"),
            ("GITHUB_TOKEN", "ghs_0123456789abcdefghijABCDEFGHIJ0123"),
            ("GITHUB_ACTOR", "o"),
            ("GITHUB_JOB", "test"),
        ];
        let w: Vec<String> = job
            .iter()
            .filter(|(k, v)| scanned(k, v))
            .map(|(k, _)| k.to_string())
            .collect();
        assert_eq!(w, ["GITHUB_ACTOR", "GITHUB_JOB"]);
        assert!(referenced(&cas, &blobs, &w).unwrap().is_empty());
    }
}
