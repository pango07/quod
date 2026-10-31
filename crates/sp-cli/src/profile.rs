//! `stillpass profile`: which toolchain profile a repo runs under and what
//! the acceptance and bench scripts need to know about it, so that
//! knowledge stays in `sp_key::profile` and the scripts eval the output.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args;
use sp_key::Toolchain;

#[derive(Args)]
pub struct ProfileArgs {
    /// The repo checkout to detect the toolchain of.
    #[arg(long)]
    repo: PathBuf,
    /// Override detection.
    #[arg(long)]
    toolchain: Option<Toolchain>,
}

pub fn profile(args: ProfileArgs) -> anyhow::Result<()> {
    let tc = match args.toolchain {
        Some(t) => t,
        None => crate::run::detect_toolchain(&args.repo)?,
    };
    let p = tc.profile();
    let text = render(p);
    match std::io::stdout().lock().write_all(text.as_bytes()) {
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        r => Ok(r?),
    }
}

fn render(p: &sp_key::Profile) -> String {
    let words = |xs: &[&str]| xs.join(" ");
    format!(
        "toolchain={}\nsetup={}\ntest={}\nephemeral={}\nseed_dirs={}\ncomment={}\n",
        p.toolchain,
        shell_quote(p.setup),
        shell_quote(p.test),
        shell_quote(&words(p.ephemeral)),
        shell_quote(&words(p.seed_dirs)),
        shell_quote(p.comment),
    )
}

/// Single-quoted for sh.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_sh_assignments() {
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        let t = render(Toolchain::Go.profile());
        assert!(t.starts_with("toolchain=go\nsetup='go mod download'\ntest='go test ./...'\n"));
        assert!(t.contains("ephemeral='.stillpass/gocache .stillpass/home'\n"));
    }
}
