use std::path::PathBuf;

use crate::error::{Error, Result, SubrepoResultExt};

#[derive(Debug, Clone)]
pub struct RepoState {
    pub workdir: PathBuf,
    pub head_branch: String,
    pub head_commit: Option<gix::ObjectId>,
}

pub fn discover_repo() -> Result<gix::Repository> {
    let cwd = std::env::current_dir()?;
    let repo = gix::discover(cwd).into_subrepo_result()?;
    Ok(repo)
}

pub fn ensure_repo_is_ready(
    repo: &gix::Repository,
    command: &str,
    require_head_commit: bool,
    require_clean_worktree: bool,
) -> Result<RepoState> {
    let workdir = repo
        .workdir()
        .ok_or_else(|| Error::user(format!("Can't 'subrepo {command}' outside a working tree.")))?
        .to_path_buf();

    let prefix = repo.prefix().into_subrepo_result()?;
    if let Some(prefix) = prefix {
        if !prefix.as_os_str().is_empty() {
            let mut comps = prefix.components();
            if matches!(
                comps.next(),
                Some(std::path::Component::Normal(s)) if s == std::ffi::OsStr::new(".git")
            ) {
                return Err(Error::user(format!(
                    "Can't 'subrepo {command}' outside a working tree."
                )));
            }

            return Err(Error::user(
                "Need to run subrepo command from top level directory of the repo.".to_string(),
            ));
        }
    }

    let head_name = repo.head_name().into_subrepo_result()?;
    let head_name =
        head_name.ok_or_else(|| Error::user("Must be on a branch to run this command."))?;

    let head_ref_full = head_name.as_bstr().to_string();
    let head_branch = head_ref_full
        .strip_prefix("refs/heads/")
        .unwrap_or(&head_ref_full)
        .to_string();

    if head_branch.starts_with("subrepo/") {
        return Err(Error::user(format!(
            "Can't '{command}' while subrepo branch is checked out."
        )));
    }

    let head_commit = repo.head_commit().ok().map(|c| c.id);

    if require_head_commit && head_commit.is_none() {
        return Err(Error::user("Must be on a branch to run this command."));
    }

    if require_clean_worktree && head_commit.is_some() {
        let dirty = repo.is_dirty().into_subrepo_result()?;
        if dirty {
            return Err(Error::user(format!(
                "Can't {command} subrepo. Working tree has changes. ({})",
                workdir.display()
            )));
        }
    }

    Ok(RepoState {
        workdir,
        head_branch,
        head_commit,
    })
}
