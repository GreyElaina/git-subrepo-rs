use std::{
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

use crate::{
    error::{Error, Result},
    VERSION,
};

pub struct GitOutput {
    pub status: std::process::ExitStatus,
    pub stdout: String,
    #[allow(dead_code)]
    pub stderr: String,
}

fn git_command(cwd: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_TERMINAL_PROMPT", "0")
        // `git subrepo` may run inside other git tooling. Avoid inheriting those.
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        .env_remove("GIT_COMMON_DIR")
        // Keep `FILTER_BRANCH_SQUELCH_WARNING` aligned with upstream behavior.
        .env("FILTER_BRANCH_SQUELCH_WARNING", "1")
        // Match upstream version in commit messages if it ever matters.
        .env("GIT_SUBREPO_CMDVER", VERSION);
    cmd
}

pub fn run_raw(cwd: &Path, args: &[&str]) -> Result<Output> {
    let out = git_command(cwd, args).output()?;
    Ok(out)
}

pub fn run(cwd: &Path, args: &[&str]) -> Result<GitOutput> {
    let out = run_raw(cwd, args)?;
    Ok(GitOutput {
        status: out.status,
        stdout: String::from_utf8_lossy(&out.stdout).to_string(),
        stderr: String::from_utf8_lossy(&out.stderr).to_string(),
    })
}

pub fn command_failed(args: &[&str]) -> String {
    format!("Command failed: 'git {}'.", args.join(" "))
}

pub fn run_or_command_failed(cwd: &Path, args: &[&str]) -> Result<GitOutput> {
    let out = run(cwd, args)?;
    if !out.status.success() {
        return Err(Error::user(command_failed(args)));
    }
    Ok(out)
}

pub fn run_checked_stdout(cwd: &Path, args: &[&str]) -> Result<String> {
    let out = run_or_command_failed(cwd, args)?;
    Ok(out.stdout.trim().to_string())
}

pub fn git_common_dir_str(cwd: &Path) -> Result<String> {
    run_checked_stdout(cwd, &["rev-parse", "--git-common-dir"])
}

pub fn git_common_dir_path(cwd: &Path) -> Result<PathBuf> {
    let raw = git_common_dir_str(cwd)?;
    let p = PathBuf::from(raw.trim());
    Ok(if p.is_absolute() { p } else { cwd.join(p) })
}

pub fn detect_remote_head_branch(cwd: &Path, remote: &str) -> Result<String> {
    let out = run_or_command_failed(cwd, &["ls-remote", "--symref", remote])?;
    for line in out.stdout.lines() {
        // Example: "ref: refs/heads/master\tHEAD"
        let mut parts = line.split_whitespace();
        let Some(first) = parts.next() else { continue };
        if first != "ref:" {
            continue;
        }
        let Some(full_ref) = parts.next() else {
            continue;
        };
        let Some(kind) = parts.next() else { continue };
        if kind != "HEAD" {
            continue;
        }
        if let Some(branch) = full_ref.strip_prefix("refs/heads/") {
            return Ok(branch.to_string());
        }
    }

    Err(Error::user("Problem finding remote default head branch."))
}

pub fn init_default_branch(repo: &gix::Repository) -> Result<String> {
    use gix::bstr::ByteSlice;

    let v = repo
        .config_snapshot()
        .string("init.defaultbranch")
        .map(|s| s.to_str_lossy().into_owned());

    Ok(v.filter(|s| !s.is_empty())
        .unwrap_or_else(|| "master".to_string()))
}
