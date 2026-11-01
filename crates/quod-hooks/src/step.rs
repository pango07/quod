//! What a `run_script_step` is: a `run:` script, a JavaScript action (and
//! which), or a bare command; how it is keyed; whether it may need the
//! network; and why it cannot replay (taint). Pure classification apart
//! from reading the step's script: no execution here.

use std::collections::BTreeMap;

use anyhow::Context;

use crate::job::Job;
use crate::protocol::ScriptStepArgs;
use crate::trust::Policy;

/// `actions/checkout` inputs the CAS checkout honours; anything else makes
/// the step run the real action instead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckoutInputs {
    pub repository: String,
    /// The ref as given (may be empty: the workflow's own commit).
    pub reference: String,
    pub token: Option<String>,
    pub server_url: String,
    /// GITHUB_SHA: the commit when `repository` is the workflow's repo.
    pub github_sha: String,
    /// GITHUB_REF: the `ref` output for the workflow's own commit.
    pub github_ref: String,
    pub github_repository: String,
}

/// A JavaScript action the runner downloaded: `owner/repo@ref`, and the
/// container path of its `<actions dir>/owner/repo/ref` directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    pub name: String,
    pub root: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `actions/checkout`'s main step with inputs the CAS checkout serves.
    Checkout(CheckoutInputs),
    /// `actions/checkout`'s post step (credential cleanup): nothing to do.
    CheckoutPost,
    /// Another JavaScript action.
    Action(Action),
    /// A `run:` step: its script text, and the argument index of the
    /// script's path in the runner's command.
    Script { text: String, arg: usize },
    /// Anything else the runner asked for.
    Command,
}

/// One classified step.
#[derive(Clone, Debug)]
pub struct Step {
    pub kind: Kind,
    /// The key's command: the entry point and its arguments, with a script
    /// path under RUNNER_TEMP replaced by the script's text (the path is
    /// fresh every run; the text is the step). The hooks append the content
    /// identity of runner-provided files the step executes (an action's
    /// directory, node from the runner's externals).
    pub cmd: Vec<String>,
    /// One line for the summary.
    pub label: String,
    /// Why the step can never replay; empty when it can. A tainted step
    /// runs with everything the runner gives a step (its mounts, the whole
    /// environment, the network) and is never recorded.
    pub taint: Vec<String>,
    /// The step may need the network (a JavaScript action, or a script
    /// matching a network rule). It runs offline first, like every keyed
    /// step; if it fails offline it runs again with the network, tainted.
    pub retry: Option<String>,
}

impl Step {
    pub fn tainted(&self) -> bool {
        !self.taint.is_empty()
    }

    /// The label for events.jsonl: a script's first line may hold anything
    /// (an inline secret), so a script is logged by its hash.
    pub fn log_label(&self) -> String {
        match &self.kind {
            Kind::Script { text, .. } => {
                format!("run: {}", quod_cas::Hash::of(text.as_bytes()).short())
            }
            _ => self.label.clone(),
        }
    }
}

/// The names of the runner's per-step command files in the step env.
pub const FILE_COMMANDS: &[&str] = &[
    "GITHUB_ENV",
    "GITHUB_PATH",
    "GITHUB_OUTPUT",
    "GITHUB_STATE",
    "GITHUB_STEP_SUMMARY",
];

/// The container path of the runner's actions directory (`_actions`).
pub fn actions_dir(job: &Job) -> Option<String> {
    mount_target(job, "/_actions")
}

/// The container path of the runner's externals (node lives there).
pub fn externals_dir(job: &Job) -> Option<String> {
    mount_target(job, "/externals")
}

/// The container path of the runner's tool cache (`_tool`).
pub fn tool_dir(job: &Job) -> Option<String> {
    mount_target(job, "/_tool")
}

/// The container path of the runner's RUNNER_TEMP (`_temp`).
pub fn temp_dir(job: &Job) -> Option<String> {
    mount_target(job, "/_temp")
}

/// The container path of the runner's workflow dir (event.json).
pub fn workflow_dir(job: &Job) -> Option<String> {
    mount_target(job, "/_github_workflow")
}

fn mount_target(job: &Job, source_suffix: &str) -> Option<String> {
    job.system_mounts
        .iter()
        .find(|m| {
            m.source_volume_path
                .trim_end_matches('/')
                .ends_with(source_suffix)
        })
        .map(|m| m.target_volume_path.trim_end_matches('/').to_string())
}

/// The `<actions dir>/owner/repo/ref` directory a path lies in, with the
/// action's `owner/repo@ref` name.
pub fn action_root(actions_dir: &str, path: &str) -> Option<Action> {
    let rest = path.strip_prefix(&format!("{actions_dir}/"))?;
    let parts: Vec<&str> = rest.split('/').collect();
    if parts.len() < 3 || parts[..3].iter().any(|p| p.is_empty() || *p == "..") {
        return None;
    }
    Some(Action {
        name: format!("{}/{}@{}", parts[0], parts[1], parts[2]),
        root: format!("{actions_dir}/{}/{}/{}", parts[0], parts[1], parts[2]),
    })
}

pub fn classify(job: &Job, args: &ScriptStepArgs, policy: &Policy) -> anyhow::Result<Step> {
    let env = &args.environment_variables;
    let mut cmd = vec![args.entry_point.clone()];
    cmd.extend(args.entry_point_args.iter().cloned());
    let mut taint = Vec::new();

    // A JavaScript action: node running <actions dir>/owner/repo/ref/....js.
    let is_node = std::path::Path::new(&args.entry_point)
        .file_name()
        .is_some_and(|n| n == "node");
    if is_node
        && let Some(dir) = actions_dir(job)
        && let Some(script) = args.entry_point_args.first()
        && let Some(action) = action_root(&dir, script)
    {
        let label = format!("uses: {}", action.name);
        if action.name.starts_with("actions/checkout@") {
            if env.get("STATE_isPost").is_some_and(|v| v == "true") {
                return Ok(Step {
                    kind: Kind::CheckoutPost,
                    cmd,
                    label: format!("{label} (post)"),
                    taint,
                    retry: None,
                });
            }
            if let Some(inputs) = checkout_inputs(env) {
                return Ok(Step {
                    kind: Kind::Checkout(inputs),
                    cmd,
                    label,
                    taint,
                    retry: None,
                });
            }
            taint.push("actions/checkout with inputs the CAS checkout does not serve (path, submodules, lfs, sparse-checkout, filter, fetch-depth other than 1, fetch-tags, clean: false, ssh-key, ssh-known-hosts)".into());
        }
        let inputs: String = env
            .iter()
            .filter(|(k, _)| k.starts_with("INPUT_"))
            .map(|(_, v)| v.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        for name in mentioned(&inputs, env) {
            taint.push(format!("an input names {name}"));
        }
        if holds_github_token(&inputs) && !only_token_input(env) {
            taint.push("an input other than `token` holds a GitHub token".into());
        }
        return Ok(Step {
            kind: Kind::Action(action),
            cmd,
            label,
            taint,
            retry: Some("a JavaScript action".into()),
        });
    }

    // A run: step: the runner wrote the script under RUNNER_TEMP.
    let temp = env.get("RUNNER_TEMP").cloned().or_else(|| temp_dir(job));
    if let Some(temp) = temp {
        let prefix = format!("{}/", temp.trim_end_matches('/'));
        for i in 1..cmd.len() {
            if cmd[i].starts_with(&prefix)
                && !cmd[i][prefix.len()..].contains('/')
                && let Some(host) = job.host_path(&cmd[i])
                && host.is_file()
            {
                let text = std::fs::read_to_string(&host)
                    .with_context(|| format!("read step script {}", host.display()))?;
                cmd[i] = text.clone();
                for name in mentioned(&text, env) {
                    taint.push(format!("the script names {name}"));
                }
                if holds_github_token(&text) {
                    taint.push("the script holds a GitHub token (an inline secret)".into());
                }
                let retry = policy
                    .net_rule(&text)
                    .map(|rule| format!("matches the network rule {rule:?}"));
                let label = text
                    .lines()
                    .map(str::trim)
                    .find(|l| !l.is_empty() && !l.starts_with('#'))
                    .unwrap_or("")
                    .chars()
                    .take(72)
                    .collect::<String>();
                return Ok(Step {
                    kind: Kind::Script { text, arg: i },
                    cmd,
                    label: format!("run: {label}"),
                    taint,
                    retry,
                });
            }
        }
    }
    // A local JavaScript action (`uses: ./x` runs node on a workspace path)
    // may talk to the network like any other.
    let label = cmd.join(" ").chars().take(72).collect();
    Ok(Step {
        kind: Kind::Command,
        cmd,
        label,
        taint,
        retry: is_node.then(|| "a JavaScript action".to_string()),
    })
}

/// The names (present in `env`) that `text` mentions and that a keyed step
/// would not be given as they are: volatile names (withheld, or replaced by
/// a placeholder) and variables holding a GitHub token.
/// A substring match, deliberately: `$GITHUB_SHA`, `${GITHUB_SHA}`,
/// `process.env.GITHUB_SHA` and `env.GITHUB_SHA` all count.
pub fn mentioned(text: &str, env: &BTreeMap<String, String>) -> Vec<String> {
    env.iter()
        .filter(|(k, v)| (quod_key::is_volatile(k) || holds_github_token(v)) && names(text, k))
        .map(|(k, _)| k.clone())
        .collect()
}

/// Does `text` name `name`: an occurrence with no identifier character on
/// either side (`GITHUB_ACTIONS` does not name `GITHUB_ACTION`).
pub fn names(text: &str, name: &str) -> bool {
    let ident = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
    text.match_indices(name).any(|(i, _)| {
        !ident(text[..i].chars().next_back()) && !ident(text[i + name.len()..].chars().next())
    })
}

/// Does a keyed step go without this variable? Volatile names (their values
/// change from run to run, so they cannot be in the key, so the step must
/// not see them), except the ACTIONS_* service variables, which it gets as
/// unreachable placeholders (`placeholder`); and anything holding a
/// GitHub-issued token (one per job: keyed, it would make every run a
/// miss; and offline it opens nothing).
pub fn withheld(name: &str, value: &str) -> bool {
    (quod_key::is_volatile(name) && placeholder(name).is_none()) || holds_github_token(value)
}

/// What a keyed step gets instead of an ACTIONS_* variable: an address
/// nothing answers on, or a token that opens nothing. A step that checks
/// for the cache or artifact service finds one, as on GitHub, and the
/// moment it uses it the network probe sees the attempt and the step runs
/// again online. Absent, the step would take the no-service branch
/// quietly and record a result GitHub would never produce.
pub fn placeholder(name: &str) -> Option<&'static str> {
    if !name.starts_with("ACTIONS_") {
        return None;
    }
    Some(if name.ends_with("_URL") {
        "http://192.0.2.1/quod-unreachable/"
    } else {
        "quod-withheld"
    })
}

impl Step {
    /// Does the step run the go tool with a command that uses the build
    /// cache (build, test, vet, run, install, generate, list, tool, fix)?
    /// Only those get the scope's warm GOCACHE (the hooks' `--warm`): any
    /// other step gets none, and a step that reaches go another way (make,
    /// a script file) builds with the observed cache in the workspace.
    pub fn runs_go(&self) -> bool {
        let Kind::Script { text, .. } = &self.kind else {
            return false;
        };
        const USES_CACHE: &[&str] = &[
            "build", "test", "vet", "run", "install", "generate", "list", "tool", "fix",
        ];
        let words: Vec<&str> = text
            .split(|c: char| c.is_whitespace() || ";&|()`$".contains(c))
            .filter(|w| !w.is_empty())
            .collect();
        words
            .windows(2)
            .any(|w| (w[0] == "go" || w[0].ends_with("/go")) && USES_CACHE.contains(&w[1]))
    }
}

/// Does the name say the value is a secret? The hook is never told which
/// values are secrets (the container-hooks protocol carries no mask list,
/// confirmed in the lab), so a telling name is one of two signals it has
/// (the other is `looks_secret` on the value). Such values are salted in
/// the key and masked in everything stored.
pub fn secret_like(name: &str) -> bool {
    const SIGNS: &[&str] = &[
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "PRIVATE",
        "API_KEY",
        "APIKEY",
        "ACCESS_KEY",
        "CREDENTIAL",
        "AUTH",
        // Names the review called out that the list above missed.
        "SSH_KEY",
        "DEPLOY_KEY",
        "SIGNING_KEY",
        "PASSPHRASE",
        "GPG",
        "PGP",
        "WEBHOOK",
    ];
    let n = name.to_ascii_uppercase();
    SIGNS.iter().any(|s| n.contains(s))
}

/// Does the *value* look like a secret, whatever its name? A name-independent
/// signal for a mis-named secret (DEPLOY_KEY, a bare API token): a PEM private
/// key or certificate, or one opaque high-entropy token. Deliberately narrow,
/// to leave benign high-entropy values a step really carries alone: a value
/// with a path separator (`/`) is not a token (a `PATH`, a URL), and a hex
/// digest or a UUID (hex and dashes only) is a public identifier, not a
/// secret (`hashFiles()` cache keys, commit ids, run ids, go.sum hashes); a
/// hex API token is missed by this, a bounded residual.
/// A slash-bearing or low-entropy secret under a plain name is a residual
/// this bounds rather than closes (see DECISIONS; the durable fix is
/// per-scope blob encryption). Masking a value this flags by mistake only
/// ever makes a step not replay, never records a wrong result.
pub fn looks_secret(value: &str) -> bool {
    let v = value.trim();
    if v.contains("PRIVATE KEY") || v.contains("BEGIN OPENSSH") {
        return true;
    }
    if v.len() < 24 || v.contains('/') || v.bytes().any(|b| b.is_ascii_whitespace()) {
        return false;
    }
    if !v
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b'_' | b'-' | b'.' | b'+'))
    {
        return false;
    }
    // A hex digest or a UUID (only hex digits and dashes): a public
    // identifier, not a secret.
    if v.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
        return false;
    }
    let has_alpha = v.bytes().any(|b| b.is_ascii_alphabetic());
    let has_digit = v.bytes().any(|b| b.is_ascii_digit());
    has_alpha && has_digit && shannon_bits_per_char(v) >= 3.5
}

/// Shannon entropy of `s` in bits per byte (0 for empty or single-symbol).
fn shannon_bits_per_char(s: &str) -> f64 {
    let mut counts = [0u32; 256];
    for b in s.bytes() {
        counts[b as usize] += 1;
    }
    let n = s.len() as f64;
    -counts
        .iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f64 / n;
            p * p.log2()
        })
        .sum::<f64>()
}

/// Whether the only input holding a GitHub token is `token` (setup-go's
/// default `${{ github.token }}`): that one is withheld like any token.
fn only_token_input(env: &BTreeMap<String, String>) -> bool {
    env.iter()
        .filter(|(k, v)| k.starts_with("INPUT_") && holds_github_token(v))
        .all(|(k, _)| k == "INPUT_TOKEN")
}

/// Does `value` contain a GitHub-issued token (`ghs_...`, `ghp_...`, ...)?
pub fn holds_github_token(value: &str) -> bool {
    const PREFIXES: &[&str] = &["ghs_", "ghp_", "gho_", "ghu_", "ghr_", "github_pat_"];
    PREFIXES.iter().any(|p| {
        value.match_indices(p).any(|(i, _)| {
            let start = i == 0 || !value.as_bytes()[i - 1].is_ascii_alphanumeric();
            let tail = value[i + p.len()..]
                .bytes()
                .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
                .count();
            start && tail >= 20
        })
    })
}

/// The checkout the CAS can serve, or `None` when an input asks for more
/// than a tree at a commit.
fn checkout_inputs(env: &BTreeMap<String, String>) -> Option<CheckoutInputs> {
    let input = |n: &str| env.get(&format!("INPUT_{n}")).map(|s| s.trim().to_string());
    let plain = |n: &str, dflt: &str| input(n).is_none_or(|v| v.is_empty() || v == dflt);
    if !plain("PATH", ".")
        || !plain("SUBMODULES", "false")
        || !plain("LFS", "false")
        || !plain("SPARSE-CHECKOUT", "")
        || !plain("FILTER", "")
        || !plain("FETCH-DEPTH", "1")
        || !plain("FETCH-TAGS", "false")
        || !plain("CLEAN", "true")
        || !plain("SSH-KEY", "")
        || !plain("SSH-KNOWN-HOSTS", "")
    {
        return None;
    }
    let github_repository = env.get("GITHUB_REPOSITORY")?.clone();
    let server_url = input("GITHUB-SERVER-URL")
        .filter(|s| !s.is_empty())
        .or_else(|| env.get("GITHUB_SERVER_URL").cloned())
        .unwrap_or_else(|| "https://github.com".into());
    Some(CheckoutInputs {
        repository: input("REPOSITORY")
            .filter(|r| !r.is_empty())
            .unwrap_or_else(|| github_repository.clone()),
        reference: input("REF").unwrap_or_default(),
        token: input("TOKEN").filter(|t| !t.is_empty()),
        server_url,
        github_sha: env.get("GITHUB_SHA").cloned().unwrap_or_default(),
        github_ref: env.get("GITHUB_REF").cloned().unwrap_or_default(),
        github_repository,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{Container, Mount};

    fn job(tmp: &std::path::Path) -> Job {
        let work = tmp.join("_work");
        std::fs::create_dir_all(work.join("_temp")).unwrap();
        std::fs::create_dir_all(work.join("_actions/actions/checkout/abc/dist")).unwrap();
        let m = |s: &str, t: &str| Mount {
            source_volume_path: work.join(s).to_string_lossy().into_owned(),
            target_volume_path: t.into(),
            read_only: false,
        };
        let c = Container {
            image: Some("golang@sha256:x".into()),
            working_directory: Some("/__w/o/r".into()),
            system_mount_volumes: vec![
                m("", "/__w"),
                m("_temp", "/__w/_temp"),
                m("_actions", "/__w/_actions"),
            ],
            ..Container::default()
        };
        Job::create(
            &tmp.join("state"),
            &c,
            None,
            quod_key::Platform::test_aarch64(),
        )
        .unwrap()
    }

    fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn run_step_is_keyed_by_its_text_and_tainted_by_volatile_names() {
        let tmp = tempfile::tempdir().unwrap();
        let job = job(tmp.path());
        let script = tmp.path().join("_work/_temp/abc.sh");
        std::fs::write(&script, "# comment\ngo test ./...\n").unwrap();
        let args = ScriptStepArgs {
            entry_point: "bash".into(),
            entry_point_args: vec!["-e".into(), "/__w/_temp/abc.sh".into()],
            environment_variables: env(&[
                ("RUNNER_TEMP", "/__w/_temp"),
                ("GITHUB_SHA", "a"),
                (
                    "GITHUB_OUTPUT",
                    "/__w/_temp/_runner_file_commands/set_output_x",
                ),
                ("GH_TOKEN", "ghs_0123456789abcdefghijABCDEFGHIJ"),
            ]),
            prepend_path: vec![],
            working_directory: "/__w/o/r".into(),
        };
        let s = classify(&job, &args, &Policy::default()).unwrap();
        assert_eq!(s.cmd, ["bash", "-e", "# comment\ngo test ./...\n"]);
        assert_eq!(s.label, "run: go test ./...");
        assert!(
            matches!(s.kind, Kind::Script { ref text, arg: 2 } if text.starts_with("# comment"))
        );
        assert!(!s.tainted() && s.retry.is_none());

        std::fs::write(&script, "echo $GITHUB_SHA > out\n").unwrap();
        let s = classify(&job, &args, &Policy::default()).unwrap();
        assert_eq!(s.taint, ["the script names GITHUB_SHA"]);

        // The command files are pinned by the hooks, so naming one is fine;
        // naming a variable that holds a token is not.
        std::fs::write(
            &script,
            "echo x=1 >> \"$GITHUB_OUTPUT\"\ngh api -H \"$GH_TOKEN\"\n",
        )
        .unwrap();
        let s = classify(&job, &args, &Policy::default()).unwrap();
        assert_eq!(s.taint, ["the script names GH_TOKEN"]);

        std::fs::write(&script, "go mod download\n").unwrap();
        let s = classify(&job, &args, &Policy::default()).unwrap();
        assert!(!s.tainted());
        assert_eq!(
            s.retry.as_deref(),
            Some("matches the network rule \"go mod download\"")
        );
    }

    #[test]
    fn tokens_are_recognized() {
        assert!(holds_github_token("ghs_0123456789abcdefghijABCDEFGHIJ"));
        assert!(holds_github_token("x ghp_0123456789abcdefghijABCD y"));
        assert!(holds_github_token(
            "github_pat_11ABCDEFG0123456789_abcdefghij"
        ));
        assert!(!holds_github_token("ghs_short"));
        assert!(!holds_github_token("xghs_0123456789abcdefghijABCDEFGHIJ"));
        assert!(!holds_github_token("go.mod"));
        assert!(withheld("GITHUB_RUN_ID", "1"));
        assert!(withheld(
            "INPUT_TOKEN",
            "ghs_0123456789abcdefghijABCDEFGHIJ"
        ));
        assert!(!withheld("INPUT_GO-VERSION-FILE", "go.mod"));
    }

    #[test]
    fn which_steps_run_go() {
        let script = |t: &str| Step {
            kind: Kind::Script {
                text: t.into(),
                arg: 1,
            },
            cmd: vec![],
            label: String::new(),
            taint: vec![],
            retry: None,
        };
        for yes in [
            "go test ./...",
            "go build -v ./cmd/gh",
            "set -e\ncd x && go vet ./...",
            "/usr/local/go/bin/go run ./script",
            "make; go install ./cmd/x",
        ] {
            assert!(script(yes).runs_go(), "{yes}");
        }
        for no in [
            "go mod download",
            "go env GOVERSION",
            "make test",
            "cargo build",
            "golangci-lint run",
        ] {
            assert!(!script(no).runs_go(), "{no}");
        }
    }

    #[test]
    fn secrets_by_name_and_by_value() {
        // Names the review called out, beyond the original list.
        assert!(secret_like("DEPLOY_KEY"));
        assert!(secret_like("SSH_KEY"));
        assert!(secret_like("MY_GPG_PRIVATE"));
        assert!(secret_like("RELEASE_PASSPHRASE"));
        assert!(secret_like("SLACK_WEBHOOK"));
        // Not every name with these letters: a cache key input is not one.
        assert!(!secret_like("INPUT_KEY"));
        assert!(!secret_like("PATH"));

        // Value-shaped secrets, whatever the name.
        assert!(looks_secret(
            "-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END"
        ));
        assert!(looks_secret(" key with a PRIVATE KEY inside"));
        assert!(looks_secret("ghp_aB3kZ9qR7tN2wX5vY8uJ1mD4cF6hL0pS2eG")); // opaque token
        assert!(looks_secret("AKIA1234567890ABCDEFXYZ9")); // 24, mixed
        // Benign values a step really carries must not be flagged.
        assert!(!looks_secret(
            "/usr/local/go/bin:/__w/_tool/go/1.27.3/arm64/bin"
        ));
        assert!(!looks_secret("-mod=mod -count=1")); // has spaces
        assert!(!looks_secret("linux")); // too short
        assert!(!looks_secret("go1.27.3")); // short, low entropy
        assert!(!looks_secret("aaaaaaaaaaaaaaaaaaaaaaaaaaaa")); // low entropy
        assert!(!looks_secret(
            "github.com/cli/cli/v2/pkg/cmd/root/help_topic.go"
        )); // path-like, has '/'
        // A hashFiles() digest / commit id / UUID is a public identifier, not
        // a secret (cli/cli's setup-go writes GO_MOD_HASH to GITHUB_OUTPUT).
        assert!(!looks_secret(
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        )); // sha256 hex
        assert!(!looks_secret("9490d204a2ba95434153dedce99898ce9f281426")); // 40-hex commit
        assert!(!looks_secret("43f6b4de-d7cc-4f38-980b-18a3d75ad463")); // uuid
    }

    #[test]
    fn action_roots() {
        let a = action_root(
            "/__w/_actions",
            "/__w/_actions/actions/cache/v5/dist/restore/index.js",
        )
        .unwrap();
        assert_eq!(a.name, "actions/cache@v5");
        assert_eq!(a.root, "/__w/_actions/actions/cache/v5");
        assert!(action_root("/__w/_actions", "/__w/_actions/actions/cache").is_none());
        assert!(action_root("/__w/_actions", "/__w/_actions/a/../b/c").is_none());
        assert!(action_root("/__w/_actions", "/__w/o/r/x.js").is_none());
    }

    #[test]
    fn checkout_and_other_actions() {
        let tmp = tempfile::tempdir().unwrap();
        let job = job(tmp.path());
        let base = [
            ("GITHUB_REPOSITORY", "o/r"),
            ("GITHUB_SHA", "0123456789012345678901234567890123456789"),
            ("GITHUB_SERVER_URL", "https://github.com"),
            ("INPUT_TOKEN", "t"),
            ("INPUT_PATH", ""),
            ("INPUT_SUBMODULES", "false"),
            ("INPUT_LFS", "false"),
            ("INPUT_FETCH-DEPTH", "1"),
        ];
        let node = "/__e/node20/bin/node";
        let mk = |script: &str, extra: &[(&str, &str)]| ScriptStepArgs {
            entry_point: node.into(),
            entry_point_args: vec![script.into()],
            environment_variables: env(&[base.as_slice(), extra].concat()),
            prepend_path: vec![],
            working_directory: "/__w/o/r".into(),
        };
        let co = "/__w/_actions/actions/checkout/abc/dist/index.js";
        let s = classify(&job, &mk(co, &[]), &Policy::default()).unwrap();
        match &s.kind {
            Kind::Checkout(c) => {
                assert_eq!(c.repository, "o/r");
                assert_eq!(c.reference, "");
                assert_eq!(c.token.as_deref(), Some("t"));
                assert_eq!(c.github_sha, base[1].1);
            }
            k => panic!("{k:?}"),
        }
        assert_eq!(s.label, "uses: actions/checkout@abc");
        assert!(!s.tainted());

        let s = classify(
            &job,
            &mk(
                co,
                &[("INPUT_REPOSITORY", "cli/cli"), ("INPUT_REF", "deadbeef")],
            ),
            &Policy::default(),
        )
        .unwrap();
        assert!(
            matches!(&s.kind, Kind::Checkout(c) if c.repository == "cli/cli" && c.reference == "deadbeef")
        );

        let s = classify(
            &job,
            &mk(co, &[("STATE_isPost", "true")]),
            &Policy::default(),
        )
        .unwrap();
        assert_eq!(s.kind, Kind::CheckoutPost);

        // Inputs the CAS checkout cannot serve fall back to the real action, tainted.
        for extra in [
            ("INPUT_SUBMODULES", "recursive"),
            ("INPUT_FETCH-DEPTH", "0"),
        ] {
            let s = classify(&job, &mk(co, &[extra]), &Policy::default()).unwrap();
            assert!(matches!(&s.kind, Kind::Action(a) if a.name == "actions/checkout@abc"));
            assert!(s.tainted(), "{extra:?}");
        }

        // Another action is keyed and may retry with the network; one whose
        // input names a volatile variable is tainted.
        let sg = "/__w/_actions/actions/setup-go/def/dist/setup/index.js";
        let s = classify(
            &job,
            &mk(sg, &[("INPUT_GO-VERSION-FILE", "go.mod")]),
            &Policy::default(),
        )
        .unwrap();
        assert!(
            matches!(&s.kind, Kind::Action(a) if a.root == "/__w/_actions/actions/setup-go/def")
        );
        assert!(!s.tainted() && s.retry.is_some());
        assert_eq!(s.label, "uses: actions/setup-go@def");
        let s = classify(
            &job,
            &mk(sg, &[("INPUT_GO-VERSION", "${GITHUB_SHA}")]),
            &Policy::default(),
        )
        .unwrap();
        assert_eq!(s.taint, ["an input names GITHUB_SHA"]);
    }
}
