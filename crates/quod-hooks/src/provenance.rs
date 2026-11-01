//! Which of a job's variables carry secrets, read from the workflow.
//!
//! The container-hooks protocol gives the hook a step's environment and no
//! mask list, so a secret the workflow maps to a plain name
//! (`DB: ${{ secrets.DATABASE_URL }}`) looks like any other value. The
//! workflow itself says where secrets go. At a job's first step Quod reads
//! the workflow file the job runs (GITHUB_WORKFLOW_REF, at
//! GITHUB_WORKFLOW_SHA, from the repository's mirror), the local reusable
//! workflows it calls and the composite actions its steps use, and collects
//! across all of its jobs the set S of names whose value an expression
//! builds from a secret: `env:` names (workflow, job, step, the job
//! container's), and `INPUT_<NAME>` for a `with:` input of a JavaScript (or
//! docker) action. A name assigned from another name in S (`${{ env.X }}`)
//! is in S too. `secrets.GITHUB_TOKEN` (and `github.token`) are not: Quod
//! withholds GitHub tokens from keyed steps by their value.
//!
//! Some uses of a secret cannot be followed to a name. Then the job is
//! opaque: nothing it runs is recorded (it may still replay what clean jobs
//! recorded). Opaque: the secrets context as a whole (`toJSON(secrets)`,
//! `secrets[...]`), a function applied directly to `secrets.X` (one over
//! `env.X` makes its result a secret as a whole), a secret in a `run:`
//! script, an `if:`, an input of a composite action, of a local action (the
//! runner reads it from the workspace at run time, which may not be the
//! workflow's commit) or of a reusable workflow, secrets passed to a remote
//! reusable workflow (`secrets: inherit` included), a secret anywhere else,
//! and a workflow that cannot be read or parsed: the analysis fails closed.
//! Only uses at the workflow's level, in a composite action, or in the job
//! itself (GITHUB_JOB) count: another job's steps never run in this one. An
//! opaque job still classifies by S: what its steps are given stays salted,
//! masked, and out of the network surface.
//!
//! What is stored: the names, and for a value the workflow builds with
//! literal text around a secret (`Bearer ${{ secrets.T }}`) that text,
//! never a value.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use yaml_rust2::{Yaml, YamlLoader};

/// One piece of a value the workflow builds: literal text, or what an
/// expression gives (a secret's, or something else's).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Piece {
    Lit(String),
    Secret,
    Other,
}

pub type Template = Vec<Piece>;

/// The provenance of a job's secrets (see the module docs).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    /// The workflow read: `owner/repo/path@ref` and the commit.
    #[serde(default)]
    pub workflow: String,
    /// S: the names (upper-cased) whose values carry a secret, each with
    /// the ways the workflow builds it.
    #[serde(default)]
    pub names: BTreeMap<String, Vec<Template>>,
    /// Why the job is opaque, if it is.
    #[serde(default)]
    pub opaque: Option<String>,
    /// The names (upper-cased) the workflow assigns from a step's or a
    /// job's outputs (`steps.X.outputs.Y`, `needs.X.outputs.Y`): what a
    /// step computed, which may be a part it cut out of a secret.
    #[serde(default)]
    pub derived: BTreeSet<String>,
}

impl Provenance {
    pub fn opaque(workflow: impl Into<String>, why: impl Into<String>) -> Provenance {
        Provenance {
            workflow: workflow.into(),
            names: BTreeMap::new(),
            opaque: Some(why.into()),
            derived: BTreeSet::new(),
        }
    }

    /// Does the workflow map a secret to `name`? (Case-insensitively: a
    /// mistake here only ever classifies more.)
    pub fn names_secret(&self, name: &str) -> bool {
        self.names.contains_key(&name.to_ascii_uppercase())
    }

    /// The secret parts of `value`, a value of `name` built by a template
    /// with literal text or several expressions: what a step could print of
    /// it alone. Literal text is matched leftmost; a value no template
    /// matches gives none (the whole value is masked regardless).
    pub fn secret_parts(&self, name: &str, value: &str) -> Vec<String> {
        let mut out = Vec::new();
        for t in self
            .names
            .get(&name.to_ascii_uppercase())
            .into_iter()
            .flatten()
        {
            if t.len() < 2 {
                continue;
            }
            if let Some(parts) = split(t, value) {
                out.extend(parts.into_iter().filter(|p| !p.is_empty()));
            }
        }
        out.sort();
        out.dedup();
        out
    }
}

/// The parts of `value` the template's secret expressions gave.
fn split(t: &Template, value: &str) -> Option<Vec<String>> {
    let mut pos = 0;
    let mut pending: Option<bool> = None;
    let mut out = Vec::new();
    for (i, p) in t.iter().enumerate() {
        match p {
            Piece::Lit(l) => {
                let at = match pending {
                    // The last literal ends the value: matched at the end, so
                    // a secret holding the literal (`a-${{ secrets.S }}-b`
                    // with S holding `-b`) is not cut short.
                    Some(_) if i + 1 == t.len() => {
                        let at = value.len().checked_sub(l.len())?;
                        (at >= pos && value[at..] == *l).then_some(at)?
                    }
                    Some(_) => pos + value[pos..].find(l.as_str())?,
                    None if value[pos..].starts_with(l.as_str()) => pos,
                    None => return None,
                };
                if pending.take() == Some(true) {
                    out.push(value[pos..at].to_string());
                }
                pos = at + l.len();
            }
            Piece::Secret | Piece::Other => {
                if pending.is_some() {
                    // Two expressions with nothing between: the boundary is
                    // unknown; the whole remaining run is masked as one.
                    pending = Some(pending == Some(true) || *p == Piece::Secret);
                } else {
                    pending = Some(*p == Piece::Secret);
                }
            }
        }
    }
    match pending {
        Some(true) => out.push(value[pos..].to_string()),
        Some(false) => {}
        None if pos != value.len() => return None,
        None => {}
    }
    Some(out)
}

/// Where the analysis reads other files.
pub struct Sources<'a> {
    /// A file of the workflow's repository at the workflow's commit, by its
    /// repository path: a local reusable workflow, a local action's
    /// `action.yml`.
    pub local: &'a dyn Fn(&str) -> Option<String>,
    /// A remote action's `action.yml` (or `.yaml`) as the runner downloaded
    /// it, by its `uses:` (`owner/repo[/path]@ref`).
    pub remote_action: &'a dyn Fn(&str) -> Option<String>,
}

/// What one expression mentions.
#[derive(Clone, Debug, Default)]
struct Facts {
    /// `secrets.NAME` (NAME upper-cased), GITHUB_TOKEN left out.
    secrets: Vec<String>,
    /// The secrets context used other than by a name.
    whole: Option<&'static str>,
    /// `env.NAME`, upper-cased.
    env: Vec<String>,
    /// The env context used other than by a name.
    env_dynamic: bool,
    /// A function call.
    call: bool,
    /// The steps or needs context (outputs a step computed).
    outputs: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    Dot,
    LBracket,
    LParen,
    Star,
    Other,
}

fn tokens(expr: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut it = expr.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\'' => {
                // A string literal; '' is a quote inside it.
                loop {
                    match it.next() {
                        None => break,
                        Some('\'') if it.peek() == Some(&'\'') => {
                            it.next();
                        }
                        Some('\'') => break,
                        Some(_) => {}
                    }
                }
                out.push(Tok::Other);
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let mut s = String::from(c);
                while let Some(&n) = it.peek() {
                    if n.is_ascii_alphanumeric() || n == '_' || n == '-' {
                        s.push(n);
                        it.next();
                    } else {
                        break;
                    }
                }
                out.push(Tok::Ident(s));
            }
            c if c.is_ascii_digit() => {
                while it.peek().is_some_and(|n| n.is_ascii_alphanumeric()) {
                    it.next();
                }
                out.push(Tok::Other);
            }
            '.' => out.push(Tok::Dot),
            '[' => out.push(Tok::LBracket),
            '(' => out.push(Tok::LParen),
            '*' => out.push(Tok::Star),
            c if c.is_whitespace() => {}
            _ => out.push(Tok::Other),
        }
    }
    out
}

fn facts(expr: &str) -> Facts {
    let t = tokens(expr);
    let mut f = Facts::default();
    for i in 0..t.len() {
        let Tok::Ident(id) = &t[i] else { continue };
        if i > 0 && t[i - 1] == Tok::Dot {
            continue;
        }
        let next = t.get(i + 1);
        if next == Some(&Tok::LParen) {
            f.call = true;
            continue;
        }
        let member = match (next, t.get(i + 2)) {
            (Some(Tok::Dot), Some(Tok::Ident(n))) => Some(n.to_ascii_uppercase()),
            _ => None,
        };
        if id.eq_ignore_ascii_case("secrets") {
            match (member, next) {
                (Some(n), _) if n == "GITHUB_TOKEN" => {}
                (Some(n), _) => f.secrets.push(n),
                (None, Some(Tok::LBracket)) => f.whole = Some("secrets[...] indexing"),
                (None, Some(Tok::Dot)) => f.whole = Some("a secrets.* filter"),
                (None, _) => {
                    f.whole = Some("the secrets context as a whole (toJSON(secrets) or the like)")
                }
            }
        } else if id.eq_ignore_ascii_case("steps") || id.eq_ignore_ascii_case("needs") {
            f.outputs = true;
        } else if id.eq_ignore_ascii_case("env") {
            match member {
                Some(n) => f.env.push(n),
                None => f.env_dynamic = true,
            }
        }
    }
    f
}

/// A string's literal text and expressions; an `if:` is an expression even
/// without `${{ }}`.
#[derive(Clone, Debug)]
enum Seg {
    Lit(String),
    Expr(Facts),
}

fn segments(s: &str, is_if: bool) -> Vec<Seg> {
    if is_if && !s.contains("${{") {
        return vec![Seg::Expr(facts(s))];
    }
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(start) = rest.find("${{") {
        if start > 0 {
            out.push(Seg::Lit(rest[..start].to_string()));
        }
        let body = &rest[start + 3..];
        // The closing `}}` outside a string literal.
        let mut in_str = false;
        let mut end = None;
        let b = body.as_bytes();
        let mut i = 0;
        while i < b.len() {
            match b[i] {
                b'\'' => in_str = !in_str,
                b'}' if !in_str && b.get(i + 1) == Some(&b'}') => {
                    end = Some(i);
                    break;
                }
                _ => {}
            }
            i += 1;
        }
        match end {
            Some(e) => {
                out.push(Seg::Expr(facts(&body[..e])));
                rest = &body[e + 2..];
            }
            None => {
                // Unterminated: the runner refuses the workflow; whatever it
                // mentions is treated as an expression.
                out.push(Seg::Expr(facts(body)));
                rest = "";
            }
        }
    }
    if !rest.is_empty() {
        out.push(Seg::Lit(rest.to_string()));
    }
    out
}

#[derive(Clone, Debug)]
enum Place {
    /// An `env:` entry.
    Env(String),
    /// A `with:` input of a step's action.
    Input {
        uses: String,
        name: String,
    },
    Run,
    If,
    /// A `with:` input of a reusable workflow call.
    ReusableWith,
    /// Where a secret is fine and reaches no step (`container.credentials`,
    /// a local reusable workflow's `secrets:`).
    Allowed,
    Other,
}

#[derive(Clone, Debug)]
struct Use {
    place: Place,
    /// The job it is in (`None`: the workflow's own level, or a composite
    /// action's steps, which count for every job).
    job: Option<String>,
    at: String,
    segs: Vec<Seg>,
}

#[derive(Clone, Debug, PartialEq)]
enum ActionKind {
    /// JavaScript or docker: inputs reach the step as INPUT_<NAME>.
    Env,
    Composite,
    Unknown(String),
}

struct Walker<'a> {
    src: &'a Sources<'a>,
    uses: Vec<Use>,
    problems: Vec<(Option<String>, String)>,
    /// The job being walked.
    cur_job: Option<String>,
    /// Every job id seen with `steps:` (the workflow's and its local
    /// callees'): the jobs whose steps can run on this runner.
    job_ids: BTreeSet<String>,
    /// The remote reusable workflows the tree calls: their jobs run under
    /// the caller's GITHUB_WORKFLOW_REF, and Quod cannot read them.
    remote_calls: Vec<String>,
    kinds: BTreeMap<String, ActionKind>,
    seen: BTreeSet<String>,
}

fn key_str(k: &Yaml) -> String {
    match k {
        Yaml::String(s) => s.clone(),
        Yaml::Integer(i) => i.to_string(),
        Yaml::Boolean(b) => b.to_string(),
        Yaml::Real(r) => r.clone(),
        _ => "?".to_string(),
    }
}

fn parse(text: &str) -> Result<Yaml, String> {
    let mut docs = YamlLoader::load_from_str(text).map_err(|e| e.to_string())?;
    match docs.len() {
        1 => Ok(docs.remove(0)),
        0 => Err("empty".into()),
        n => Err(format!("{n} YAML documents")),
    }
}

/// Merge keys (`<<:`) and other values the runner's YAML reader treats in
/// its own way: not followed.
fn odd_construct(y: &Yaml) -> Option<&'static str> {
    match y {
        Yaml::Hash(h) => {
            for (k, v) in h {
                if k.as_str() == Some("<<") {
                    return Some("a YAML merge key (<<)");
                }
                if let Some(w) = odd_construct(k).or_else(|| odd_construct(v)) {
                    return Some(w);
                }
            }
            None
        }
        Yaml::Array(a) => a.iter().find_map(odd_construct),
        Yaml::BadValue => Some("a YAML value that cannot be read"),
        _ => None,
    }
}

impl Walker<'_> {
    fn problem(&mut self, why: String) {
        self.problems.push((self.cur_job.clone(), why));
    }

    fn add(&mut self, place: Place, at: &str, s: &str, is_if: bool) {
        let segs = segments(s, is_if);
        if segs.iter().any(|s| matches!(s, Seg::Expr(_))) {
            self.uses.push(Use {
                place,
                job: self.cur_job.clone(),
                at: at.to_string(),
                segs,
            });
        }
    }

    /// Every string under `y` (keys too), at `place`.
    fn generic(&mut self, at: &str, y: &Yaml, place: &Place) {
        match y {
            Yaml::String(s) => {
                let is_if = at.ends_with(".if");
                let p = if is_if { Place::If } else { place.clone() };
                self.add(p, at, s, is_if);
            }
            Yaml::Hash(h) => {
                for (k, v) in h {
                    let ks = key_str(k);
                    if let Yaml::String(s) = k {
                        self.add(Place::Other, &format!("{at} (key)"), s, false);
                    }
                    self.generic(&format!("{at}.{ks}"), v, place);
                }
            }
            Yaml::Array(a) => {
                for (i, v) in a.iter().enumerate() {
                    self.generic(&format!("{at}[{i}]"), v, place);
                }
            }
            _ => {}
        }
    }

    fn env_map(&mut self, at: &str, y: &Yaml) {
        match y {
            Yaml::Hash(h) => {
                for (k, v) in h {
                    let name = key_str(k);
                    let a = format!("{at}.{name}");
                    match v {
                        Yaml::String(s) => self.add(Place::Env(name), &a, s, false),
                        Yaml::Hash(_) | Yaml::Array(_) => self.generic(&a, v, &Place::Other),
                        _ => {}
                    }
                }
            }
            _ => self.generic(at, y, &Place::Other),
        }
    }

    fn workflow(&mut self, at: &str, y: &Yaml) {
        let Some(h) = y.as_hash() else {
            self.problem(format!("{at} is not a workflow (not a mapping)"));
            return;
        };
        if y["jobs"].as_hash().is_none() {
            self.problem(format!("{at} has no jobs"));
        }
        for (k, v) in h {
            let ks = key_str(k);
            let a = format!("{at}:{ks}");
            match ks.as_str() {
                "env" => self.env_map(&a, v),
                "jobs" => {
                    if let Some(jobs) = v.as_hash() {
                        for (id, job) in jobs {
                            let id = key_str(id);
                            if job["steps"].as_vec().is_some() {
                                self.job_ids.insert(id.clone());
                            }
                            let outer = self.cur_job.replace(id.clone());
                            self.job(&format!("{a}.{id}"), job);
                            self.cur_job = outer;
                        }
                    }
                }
                _ => self.generic(&a, v, &Place::Other),
            }
        }
    }

    fn job(&mut self, at: &str, y: &Yaml) {
        let Some(h) = y.as_hash() else {
            self.problem(format!("{at} is not a mapping"));
            return;
        };
        let uses = y["uses"].as_str().map(str::to_string);
        let local = uses.as_deref().is_some_and(|u| u.starts_with("./"));
        if let Some(u) = uses.as_deref().filter(|_| !local) {
            self.remote_calls.push(u.to_string());
        }
        for (k, v) in h {
            let ks = key_str(k);
            let a = format!("{at}.{ks}");
            match ks.as_str() {
                "env" => self.env_map(&a, v),
                "steps" => self.steps(&a, v),
                "container" if v.as_hash().is_some() => {
                    for (ck, cv) in v.as_hash().unwrap() {
                        let cks = key_str(ck);
                        let ca = format!("{a}.{cks}");
                        match cks.as_str() {
                            "env" => self.env_map(&ca, cv),
                            "credentials" => self.generic(&ca, cv, &Place::Allowed),
                            _ => self.generic(&ca, cv, &Place::Other),
                        }
                    }
                }
                // Service containers are refused at prepare_job; their env
                // and credentials reach no step.
                "services" => self.generic(&a, v, &Place::Allowed),
                // What a reusable workflow call passes reaches the callee's
                // jobs, not this one: it counts for every job.
                "with" if uses.is_some() => {
                    let outer = self.cur_job.take();
                    self.generic(&a, v, &Place::ReusableWith);
                    self.cur_job = outer;
                }
                "secrets" if uses.is_some() => {
                    let u = uses.as_deref().unwrap();
                    match v {
                        Yaml::String(s) if s == "inherit" && !local => self.problem(
                            format!("secrets: inherit passes every secret to the remote reusable workflow {u} ({a})"),
                        ),
                        Yaml::String(s) if s == "inherit" => {}
                        Yaml::Hash(m) if !local && !m.is_empty() => self.problem(format!("secrets passed to the remote reusable workflow {u} ({a})")),
                        _ => self.generic(&a, v, &Place::Allowed),
                    }
                }
                "uses" => {}
                _ => self.generic(&a, v, &Place::Other),
            }
        }
        if let Some(u) = uses.filter(|_| local) {
            let path = u.trim_start_matches("./").to_string();
            if !self.seen.insert(format!("workflow {path}")) {
                return;
            }
            match (self.src.local)(&path).map(|t| parse(&t)) {
                Some(Ok(callee)) => match odd_construct(&callee) {
                    Some(w) => self.problem(format!("{w} in {path}")),
                    None => {
                        let outer = self.cur_job.take();
                        self.workflow(&path, &callee);
                        self.cur_job = outer;
                    }
                },
                Some(Err(e)) => self.problem(format!(
                    "the reusable workflow {path} cannot be parsed ({e})"
                )),
                None => self.problem(format!("the reusable workflow {path} cannot be read")),
            }
        }
    }

    fn steps(&mut self, at: &str, y: &Yaml) {
        let Some(steps) = y.as_vec() else {
            self.generic(at, y, &Place::Other);
            return;
        };
        for (i, step) in steps.iter().enumerate() {
            let a = format!("{at}[{i}]");
            let Some(h) = step.as_hash() else {
                self.generic(&a, step, &Place::Other);
                continue;
            };
            let uses = step["uses"].as_str().map(str::to_string);
            for (k, v) in h {
                let ks = key_str(k);
                let sa = format!("{a}.{ks}");
                match (ks.as_str(), v) {
                    ("env", _) => self.env_map(&sa, v),
                    ("run", Yaml::String(s)) => self.add(Place::Run, &sa, s, false),
                    ("if", Yaml::String(s)) => self.add(Place::If, &sa, s, true),
                    ("with", Yaml::Hash(m)) if uses.is_some() => {
                        for (n, val) in m {
                            let name = key_str(n);
                            let place = Place::Input {
                                uses: uses.clone().unwrap(),
                                name: name.clone(),
                            };
                            match val {
                                Yaml::String(s) => {
                                    self.add(place, &format!("{sa}.{name}"), s, false)
                                }
                                _ => self.generic(&format!("{sa}.{name}"), val, &Place::Other),
                            }
                        }
                    }
                    ("uses", _) => {}
                    _ => self.generic(&sa, v, &Place::Other),
                }
            }
            if let Some(u) = uses {
                self.action(&u);
            }
        }
    }

    /// The kind of the action `uses` names, read once; a composite's own
    /// steps are walked too (its env can copy a secret to another name).
    fn action(&mut self, uses: &str) -> ActionKind {
        if let Some(k) = self.kinds.get(uses) {
            return k.clone();
        }
        // Recursion guard: a composite that uses itself.
        self.kinds
            .insert(uses.to_string(), ActionKind::Unknown("being read".into()));
        let kind = if uses.starts_with("docker://") {
            ActionKind::Env
        } else {
            let text = if let Some(p) = uses.strip_prefix("./") {
                let p = p.trim_end_matches('/');
                let base = if p.is_empty() || p == "." {
                    String::new()
                } else {
                    format!("{p}/")
                };
                (self.src.local)(&format!("{base}action.yml"))
                    .or_else(|| (self.src.local)(&format!("{base}action.yaml")))
            } else {
                (self.src.remote_action)(uses)
            };
            match text.map(|t| parse(&t)) {
                None => ActionKind::Unknown("its action.yml cannot be read".to_string()),
                Some(Err(e)) => {
                    ActionKind::Unknown(format!("its action.yml cannot be parsed ({e})"))
                }
                Some(Ok(y)) => {
                    let using = y["runs"]["using"]
                        .as_str()
                        .unwrap_or_default()
                        .to_ascii_lowercase();
                    if using == "composite" {
                        if let Some(w) = odd_construct(&y) {
                            self.problem(format!("{w} in the action {uses}"));
                        }
                        let outer = self.cur_job.take();
                        self.steps(&format!("{uses}:runs.steps"), &y["runs"]["steps"]);
                        self.cur_job = outer;
                        ActionKind::Composite
                    } else if using.starts_with("node") || using == "docker" {
                        ActionKind::Env
                    } else {
                        ActionKind::Unknown(format!("runs.using is {using:?}"))
                    }
                }
            }
        };
        self.kinds.insert(uses.to_string(), kind.clone());
        kind
    }
}

fn input_env(name: &str) -> String {
    format!("INPUT_{}", name.replace(' ', "_").to_ascii_uppercase())
}

fn exprs(u: &Use) -> impl Iterator<Item = &Facts> {
    u.segs.iter().filter_map(|s| match s {
        Seg::Expr(f) => Some(f),
        Seg::Lit(_) => None,
    })
}

fn carries(f: &Facts, s: &BTreeMap<String, Vec<Template>>) -> bool {
    !f.secrets.is_empty() || f.env.iter().any(|n| s.contains_key(n))
}

fn template(u: &Use, s: &BTreeMap<String, Vec<Template>>) -> Template {
    u.segs
        .iter()
        .map(|seg| match seg {
            Seg::Lit(l) => Piece::Lit(l.clone()),
            Seg::Expr(f) if carries(f, s) => Piece::Secret,
            Seg::Expr(_) => Piece::Other,
        })
        .collect()
}

/// Analyzes the workflow at `path` (its text `text`); see the module docs.
/// `workflow` in the result is left for the caller to fill.
/// `job` is the job's id (GITHUB_JOB): a use of a secret that cannot be
/// followed makes the job opaque when it is at the workflow's level, in a
/// composite action, or in that job (any job, when `job` is unknown or names
/// none of the workflow's): another job's steps never run in this one, and
/// a secret given to one of them reaches no step here. S itself is the union
/// over every job, which only ever classifies more.
pub fn analyze(path: &str, text: &str, src: &Sources, job: Option<&str>) -> Provenance {
    let y = match parse(text) {
        Ok(y) => y,
        Err(e) => {
            return Provenance::opaque("", format!("the workflow {path} cannot be parsed ({e})"));
        }
    };
    if let Some(w) = odd_construct(&y) {
        return Provenance::opaque("", format!("{w} in the workflow {path}"));
    }
    let mut w = Walker {
        src,
        uses: Vec::new(),
        problems: Vec::new(),
        cur_job: None,
        job_ids: BTreeSet::new(),
        remote_calls: Vec::new(),
        kinds: BTreeMap::new(),
        seen: BTreeSet::new(),
    };
    w.seen.insert(format!("workflow {path}"));
    w.workflow(path, &y);
    // S, to a fixed point: a name assigned from a name in S is in S.
    let mut s: BTreeMap<String, Vec<Template>> = BTreeMap::new();
    loop {
        let mut changed = false;
        for u in &w.uses {
            let name = match &u.place {
                Place::Env(n) => n.to_ascii_uppercase(),
                Place::Input { uses, name } if w.kinds.get(uses) == Some(&ActionKind::Env) => {
                    input_env(name)
                }
                _ => continue,
            };
            if !exprs(u).any(|f| carries(f, &s)) {
                continue;
            }
            let t = template(u, &s);
            let ts = s.entry(name).or_default();
            if !ts.contains(&t) {
                ts.push(t);
                ts.sort();
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // Which job this is must be certain, or every use counts and the job
    // is opaque: a GITHUB_JOB that is no job with steps here is a job of a
    // remote reusable workflow (it runs under the caller's
    // GITHUB_WORKFLOW_REF) or of something Quod did not read; and once the
    // tree calls any remote reusable workflow, a job id can name one of its
    // jobs as well as one here (environment secrets reach those jobs with
    // nothing passed).
    let certain = match job {
        None => Err("GITHUB_JOB is not set".to_string()),
        Some(_) if !w.remote_calls.is_empty() => Err(format!(
            "the workflow calls the remote reusable workflow {}, whose jobs Quod cannot read (a job id here may be one of its jobs)",
            w.remote_calls[0]
        )),
        Some(j) if !w.job_ids.contains(j) => Err(format!(
            "GITHUB_JOB {j:?} is no job with steps in the workflow Quod read"
        )),
        Some(j) => Ok(j),
    };
    let applies = |tag: &Option<String>| match (tag, &certain) {
        (None, _) | (_, Err(_)) => true,
        (Some(t), Ok(j)) => t == j,
    };
    let mut opaque = certain.as_ref().err().cloned().or_else(|| {
        w.problems
            .iter()
            .find(|(tag, _)| applies(tag))
            .map(|(_, why)| why.clone())
    });
    for u in &w.uses {
        if opaque.is_some() {
            break;
        }
        if !applies(&u.job) {
            continue;
        }
        let at = &u.at;
        for f in exprs(u) {
            if let Some(why) = f.whole {
                opaque = Some(format!("{why} at {at}"));
            } else if !f.secrets.is_empty() && f.call {
                opaque = Some(format!("a function applied to a secret at {at}"));
            } else if f.env_dynamic && !s.is_empty() {
                opaque = Some(format!(
                    "the env context as a whole at {at}, while env carries secrets"
                ));
            }
            if opaque.is_some() {
                break;
            }
        }
        if opaque.is_some() {
            break;
        }
        let direct = exprs(u).any(|f| !f.secrets.is_empty());
        let any = exprs(u).any(|f| carries(f, &s));
        opaque = match &u.place {
            Place::Env(_) | Place::Allowed => None,
            Place::Input { uses, name } if any && uses.starts_with("./") => Some(format!(
                "a secret passed to the local action {uses} (input {name}), which the runner reads from the workspace at run time, not from the workflow's commit"
            )),
            Place::Input { uses, name } if any => match w.kinds.get(uses) {
                Some(ActionKind::Env) => None,
                Some(ActionKind::Composite) => Some(format!(
                    "a secret passed to the composite action {uses} (input {name}), whose steps Quod does not follow"
                )),
                Some(ActionKind::Unknown(why)) => Some(format!(
                    "a secret passed to the action {uses} (input {name}), and {why}"
                )),
                None => Some(format!(
                    "a secret passed to the action {uses} (input {name})"
                )),
            },
            Place::Input { .. } => None,
            Place::Run if any => Some(format!("a secret in a run: script ({at})")),
            Place::If if direct => Some(format!("a secret in an if: ({at})")),
            Place::ReusableWith if any => Some(format!(
                "a secret passed as an input of a reusable workflow ({at})"
            )),
            Place::Other if any => Some(format!("a secret where Quod cannot follow it ({at})")),
            _ => None,
        };
    }
    let derived = w
        .uses
        .iter()
        .filter(|u| exprs(u).any(|f| f.outputs))
        .filter_map(|u| match &u.place {
            Place::Env(n) => Some(n.to_ascii_uppercase()),
            Place::Input { uses, name } if w.kinds.get(uses) == Some(&ActionKind::Env) => {
                Some(input_env(name))
            }
            _ => None,
        })
        .collect();
    Provenance {
        derived,
        workflow: String::new(),
        // Kept when opaque too: what the job's steps are given is still
        // classified (salted, masked, kept out of the network surface).
        names: s,
        opaque,
    }
}

/// The workflow a job runs, from GITHUB_WORKFLOW_REF
/// (`owner/repo/path@ref`): `(owner/repo, path, ref)`.
pub fn workflow_ref(r: &str) -> Option<(String, String, String)> {
    let (left, git_ref) = r.rsplit_once('@')?;
    let mut parts = left.splitn(3, '/');
    let (owner, repo, path) = (parts.next()?, parts.next()?, parts.next()?);
    if owner.is_empty()
        || repo.is_empty()
        || path.is_empty()
        || path.split('/').any(|c| c.is_empty() || c == "..")
    {
        return None;
    }
    Some((
        format!("{owner}/{repo}"),
        path.to_string(),
        git_ref.to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(text: &str, local: &[(&str, &str)], remote: &[(&str, &str)]) -> Provenance {
        with_job(text, local, remote, Some("a"))
    }

    fn with_job(
        text: &str,
        local: &[(&str, &str)],
        remote: &[(&str, &str)],
        job: Option<&str>,
    ) -> Provenance {
        let l: BTreeMap<String, String> = local
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let r: BTreeMap<String, String> = remote
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let lf = |p: &str| l.get(p).cloned();
        let rf = |u: &str| r.get(u).cloned();
        analyze(
            ".github/workflows/ci.yml",
            text,
            &Sources {
                local: &lf,
                remote_action: &rf,
            },
            job,
        )
    }

    fn names(p: &Provenance) -> Vec<&str> {
        assert_eq!(p.opaque, None, "not opaque");
        p.names.keys().map(String::as_str).collect()
    }

    const JS: &str = "runs:\n  using: node20\n  main: index.js\n";
    const COMPOSITE: &str =
        "runs:\n  using: composite\n  steps:\n    - run: echo hi\n      shell: bash\n";

    #[test]
    fn env_at_every_level() {
        let p = with(
            r#"
on: push
env:
  TOP: ${{ secrets.TOP_SECRET }}
  PLAIN: hello
jobs:
  a:
    runs-on: x
    env:
      JOBLEVEL: ${{ secrets.J }}
      ALSO_PLAIN: ${{ github.sha }}
    container:
      image: golang
      env:
        CONTAINER_DSN: ${{ secrets.DSN }}
      credentials:
        password: ${{ secrets.REGISTRY }}
    steps:
      - run: ./x.sh
        env:
          db_password: ${{ secrets.DB }}
          COPY: ${{ env.JOBLEVEL }}
"#,
            &[],
            &[],
        );
        assert_eq!(
            names(&p),
            ["CONTAINER_DSN", "COPY", "DB_PASSWORD", "JOBLEVEL", "TOP"]
        );
        assert!(p.names_secret("db_password") && p.names_secret("DB_PASSWORD"));
        assert!(!p.names_secret("PLAIN") && !p.names_secret("ALSO_PLAIN"));
    }

    #[test]
    fn inputs_of_actions() {
        let wf = |uses: &str| {
            format!(
                "on: push\njobs:\n  a:\n    runs-on: x\n    steps:\n      - uses: {uses}\n        with:\n          api key: ${{{{ secrets.K }}}}\n          plain: x\n"
            )
        };
        // A JavaScript action gets it as INPUT_<NAME>.
        let p = with(&wf("o/js@v1"), &[], &[("o/js@v1", JS)]);
        assert_eq!(names(&p), ["INPUT_API_KEY"]);
        // A local one is read by the runner from the workspace at run
        // time, which may not be the workflow's commit: opaque.
        let p = with(&wf("./local-js"), &[("local-js/action.yml", JS)], &[]);
        assert!(
            p.opaque
                .as_deref()
                .is_some_and(|w| w.contains("local action")),
            "{p:?}"
        );
        // A composite one can put it anywhere: opaque.
        let p = with(&wf("o/comp@v1"), &[], &[("o/comp@v1", COMPOSITE)]);
        assert!(
            p.opaque.as_deref().is_some_and(|w| w.contains("composite")),
            "{p:?}"
        );
        // An action whose kind cannot be read: opaque, failing closed.
        let p = with(&wf("o/missing@v1"), &[], &[]);
        assert!(p.opaque.is_some(), "{p:?}");
        // No secret in its inputs: nothing to read.
        let p = with(
            "on: push\njobs:\n  a:\n    steps:\n      - uses: o/missing@v1\n        with:\n          x: ${{ github.sha }}\n",
            &[],
            &[],
        );
        assert_eq!(names(&p), Vec::<&str>::new());
    }

    #[test]
    fn reusable_workflows_local_and_remote() {
        let callee = "on:\n  workflow_call:\n    secrets:\n      S:\n        required: true\njobs:\n  b:\n    runs-on: x\n    steps:\n      - run: ./deploy.sh\n        env:\n          DEPLOY_CRED: ${{ secrets.S }}\n";
        let local = [(".github/workflows/deploy.yml", callee)];
        // inherit, to a local workflow: its own env names are followed.
        let p = with_job(
            "on: push\njobs:\n  call:\n    uses: ./.github/workflows/deploy.yml\n    secrets: inherit\n",
            &local,
            &[],
            Some("b"),
        );
        assert_eq!(names(&p), ["DEPLOY_CRED"]);
        let p = with_job(
            "on: push\njobs:\n  call:\n    uses: ./.github/workflows/deploy.yml\n    secrets:\n      S: ${{ secrets.PROD }}\n",
            &local,
            &[],
            Some("b"),
        );
        assert_eq!(names(&p), ["DEPLOY_CRED"]);
        // The caller job (no steps) is no job that runs here: opaque.
        let p = with_job(
            "on: push\njobs:\n  call:\n    uses: ./.github/workflows/deploy.yml\n    secrets: inherit\n",
            &local,
            &[],
            Some("call"),
        );
        assert!(p.opaque.is_some(), "{p:?}");
        // To a remote one, inherit or by name: opaque.
        for s in ["inherit", "\n      S: ${{ secrets.PROD }}"] {
            let p = with(
                &format!(
                    "on: push\njobs:\n  call:\n    uses: o/r/.github/workflows/d.yml@v1\n    secrets: {s}\n"
                ),
                &[],
                &[],
            );
            assert!(
                p.opaque
                    .as_deref()
                    .is_some_and(|w| w.contains("remote reusable")),
                "{s}: {p:?}"
            );
        }
        // A local callee that cannot be read: opaque.
        let p = with(
            "on: push\njobs:\n  call:\n    uses: ./.github/workflows/gone.yml\n",
            &[],
            &[],
        );
        assert!(p.opaque.is_some());
        // A secret as a reusable workflow's input: opaque.
        let p = with_job(
            "on: push\njobs:\n  call:\n    uses: ./.github/workflows/deploy.yml\n    with:\n      x: ${{ secrets.PROD }}\n",
            &local,
            &[],
            Some("b"),
        );
        assert!(
            p.opaque.as_deref().is_some_and(|w| w.contains("input")),
            "{p:?}"
        );
    }

    /// B1 (review of 5f0afd6): a remote reusable workflow's jobs run under
    /// the caller's GITHUB_WORKFLOW_REF; Quod parses the caller and cannot
    /// see their environment secrets (passed without `secrets:`), nor tell
    /// a callee job from a caller job of the same id.
    #[test]
    fn remote_reusable_workflows_make_every_job_opaque() {
        // (A) No secrets: passed; the callee's environment secrets.
        let p = with_job(
            "on: push\njobs:\n  test:\n    runs-on: x\n    steps:\n      - run: go test\n  release:\n    uses: org/infra/.github/workflows/deploy.yml@v1\n",
            &[],
            &[],
            Some("deploy"),
        );
        assert!(p.opaque.is_some(), "a callee job: {p:?}");
        // (B) A callee job whose id is a caller job's.
        let p = with_job(
            "on: push\njobs:\n  build:\n    runs-on: x\n    steps:\n      - run: make\n  release:\n    uses: org/infra/.github/workflows/r.yml@v1\n    secrets: inherit\n",
            &[],
            &[],
            Some("build"),
        );
        assert!(p.opaque.is_some(), "a job id collision: {p:?}");
    }

    /// M1 (review of 5f0afd6): a local callee's own workflow level counts
    /// for its jobs.
    #[test]
    fn a_local_callees_top_level_counts() {
        let callee = "on:\n  workflow_call:\n    inputs:\n      stage:\n        type: string\nenv:\n  DB_URL: ${{ secrets[format('{0}_DB_URL', inputs.stage)] }}\njobs:\n  migrate:\n    runs-on: x\n    steps:\n      - run: ./migrate.sh\n";
        let caller = "on: push\njobs:\n  call:\n    uses: ./.github/workflows/m.yml\n    with:\n      stage: prod\n    secrets: inherit\n";
        let local = [(".github/workflows/m.yml", callee)];
        for job in ["migrate", "call"] {
            let p = with_job(caller, &local, &[], Some(job));
            assert!(p.opaque.is_some(), "GITHUB_JOB={job}: {p:?}");
        }
        let callee = callee.replace(
            "${{ secrets[format('{0}_DB_URL', inputs.stage)] }}",
            "${{ toJSON(secrets) }}",
        );
        let p = with_job(
            caller,
            &[(".github/workflows/m.yml", &callee)],
            &[],
            Some("migrate"),
        );
        assert!(p.opaque.is_some(), "{p:?}");
    }

    /// The reviewer's probes that are not bypasses, kept as tests: spaced
    /// and upper-case references, quoted and folded scalars, aliases, a
    /// secret in job outputs, a whole env from a secret, duplicate keys.
    #[test]
    fn probe_cases() {
        let run = |body: &str| {
            with(
                &format!("on: push\njobs:\n  a:\n    runs-on: x\n{body}"),
                &[],
                &[("o/js@v1", JS)],
            )
        };
        let p = run(
            "    steps:\n      - uses: o/js@v1\n        with:\n          k: ${{ secrets . A }}\n",
        );
        assert_eq!(names(&p), ["INPUT_K"]);
        let p = run("    steps:\n      - run: x\n        env:\n          K: ${{ SECRETS.a }}\n");
        assert_eq!(names(&p), ["K"]);
        let p = run(
            "    steps:\n      - run: x\n        env:\n          K: \"${{ secrets.A }}\"\n          L: >-\n            ${{ secrets.B }}\n",
        );
        assert_eq!(names(&p), ["K", "L"]);
        let p = run(
            "    env:\n      X: &s ${{ secrets.A }}\n    steps:\n      - run: x\n        env:\n          Y: *s\n",
        );
        assert_eq!(names(&p), ["X", "Y"]);
        for body in [
            "    outputs:\n      t: ${{ secrets.A }}\n    steps: []\n",
            "    steps:\n      - run: x\n        env: ${{ fromJSON(secrets.A) }}\n",
        ] {
            assert!(run(body).opaque.is_some(), "{body}");
        }
        // Duplicate keys: refused by the reader, or the secret one kept.
        let p = run(
            "    steps:\n      - run: x\n        env:\n          K: ${{ secrets.A }}\n          K: plain\n",
        );
        assert!(p.opaque.is_some() || p.names_secret("K"), "{p:?}");
    }

    #[test]
    fn what_cannot_be_followed_is_opaque() {
        let opaque = |body: &str| {
            let p = with(
                &format!("on: push\njobs:\n  a:\n    runs-on: x\n{body}"),
                &[],
                &[],
            );
            p.opaque.unwrap_or_else(|| panic!("not opaque: {body}"))
        };
        assert!(
            opaque("    env:\n      ALL: ${{ toJSON(secrets) }}\n    steps: []\n")
                .contains("as a whole")
        );
        assert!(
            opaque("    env:\n      X: ${{ secrets['A'] }}\n    steps: []\n").contains("indexing")
        );
        assert!(
            opaque("    env:\n      X: ${{ format('{0}', secrets.A) }}\n    steps: []\n")
                .contains("function")
        );
        // An inline secret in a script, directly or through env.
        assert!(opaque("    steps:\n      - run: echo ${{ secrets.A }}\n").contains("run:"));
        assert!(
            opaque(
                "    env:\n      A: ${{ secrets.A }}\n    steps:\n      - run: echo ${{ env.A }}\n"
            )
            .contains("run:")
        );
        assert!(
            opaque("    steps:\n      - if: secrets.A != ''\n        run: x\n").contains("if:")
        );
        assert!(
            opaque(
                "    strategy:\n      matrix:\n        k: ['${{ secrets.A }}']\n    steps: []\n"
            )
            .contains("cannot follow")
        );
        assert!(
            opaque("    env:\n      A: ${{ secrets.A }}\n    steps:\n      - run: x\n        env:\n          B: ${{ toJSON(env) }}\n")
                .contains("env context")
        );
        // Unparsable, not a workflow, a merge key.
        for text in ["on: [push\njobs: {", "- a\n- b\n", "on: push\n"] {
            let p = with(text, &[], &[]);
            assert!(p.opaque.is_some(), "{text:?}");
        }
        let p = with(
            "on: push\nx: &e\n  A: ${{ secrets.A }}\njobs:\n  a:\n    env:\n      <<: *e\n    steps: []\n",
            &[],
            &[],
        );
        assert!(
            p.opaque.as_deref().is_some_and(|w| w.contains("merge key")),
            "{p:?}"
        );
    }

    /// What another job does with a secret does not make this one opaque
    /// (its steps never run here); S still counts every job's names, and an
    /// opaque job keeps them.
    #[test]
    fn only_this_jobs_uses_count() {
        let wf = "on: push\nenv:\n  TOP: ${{ secrets.T }}\njobs:\n  test:\n    steps:\n      - run: go test ./...\n        env:\n          APP_DSN: ${{ secrets.DSN }}\n  deploy:\n    steps:\n      - uses: docker/login-action@v3\n        with:\n          password: ${{ secrets.REG }}\n      - run: echo ${{ secrets.X }}\n";
        let p = with_job(wf, &[], &[], Some("test"));
        assert_eq!(names(&p), ["APP_DSN", "TOP"]);
        for job in [Some("deploy"), None, Some("not-a-job")] {
            let p = with_job(wf, &[], &[], job);
            assert!(p.opaque.is_some(), "{job:?}");
            assert!(
                p.names_secret("APP_DSN") && p.names_secret("TOP"),
                "{job:?}: S is kept"
            );
        }
        // At the workflow's level it counts for every job.
        let p = with_job(
            "on: push\nenv:\n  ALL: ${{ toJSON(secrets) }}\njobs:\n  test:\n    steps: []\n",
            &[],
            &[],
            Some("test"),
        );
        assert!(p.opaque.is_some());
    }

    #[test]
    fn github_tokens_are_left_out() {
        let p = with(
            "on: push\njobs:\n  a:\n    runs-on: x\n    env:\n      GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}\n      T2: ${{ github.token }}\n    steps:\n      - if: secrets.GITHUB_TOKEN != ''\n        uses: actions/checkout@v4\n        with:\n          token: ${{ secrets.github_token }}\n",
            &[],
            &[],
        );
        assert_eq!(names(&p), Vec::<&str>::new());
    }

    #[test]
    fn templates_give_the_secret_part() {
        let p = with(
            "on: push\njobs:\n  a:\n    env:\n      AUTH: Bearer ${{ secrets.T }}\n      PAIR: ${{ secrets.U }}:${{ secrets.P }}\n      WHOLE: ${{ secrets.W }}\n    steps: []\n",
            &[],
            &[],
        );
        assert_eq!(names(&p), ["AUTH", "PAIR", "WHOLE"]);
        assert_eq!(
            p.secret_parts("AUTH", "Bearer abc.def-123"),
            ["abc.def-123"]
        );
        assert_eq!(
            p.secret_parts("PAIR", "user:pass-word"),
            ["pass-word", "user"]
        );
        assert!(p.secret_parts("WHOLE", "anything").is_empty());
        assert!(p.secret_parts("AUTH", "Basic xyz").is_empty(), "no match");
        // The last literal is matched at the end (review of 5f0afd6, minor 4).
        let q = Provenance {
            names: [(
                "W".to_string(),
                vec![vec![
                    Piece::Lit("a-".into()),
                    Piece::Secret,
                    Piece::Lit("-b".into()),
                ]],
            )]
            .into(),
            ..Provenance::default()
        };
        assert_eq!(q.secret_parts("W", "a-x-b-y-b"), ["x-b-y"]);
    }

    #[test]
    fn workflow_refs() {
        assert_eq!(
            workflow_ref("o/r/.github/workflows/ci.yml@refs/heads/main"),
            Some((
                "o/r".into(),
                ".github/workflows/ci.yml".into(),
                "refs/heads/main".into()
            ))
        );
        assert_eq!(workflow_ref("o/r/../x.yml@refs/heads/main"), None);
        assert_eq!(workflow_ref("o/r@x"), None);
    }
}
