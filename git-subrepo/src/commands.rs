use std::path::{Path, PathBuf};

use gix::bstr::ByteSlice;

use git_subrepo_core::{Error, Result, SubrepoResultExt};

use git_subrepo_core::{
    git_cli,
    gitrepo::{GitRepoState, JoinMethod},
    refs::SubrepoRefs,
    remote,
    repo::{ensure_repo_is_ready, RepoState},
    subdir,
};

use gix_filter_branch as filter_branch;

pub const VERSION: &str = git_subrepo_core::VERSION;

pub struct CloneArgs {
    pub remote: String,
    pub subdir: Option<String>,
    pub branch: Option<String>,
    pub force: bool,
    pub method: JoinMethod,
    pub message: Option<String>,
    pub message_file: Option<String>,
    pub edit: bool,
}

pub fn clone(args: CloneArgs) -> Result<String> {
    let repo = git_subrepo_core::repo::discover_repo()?;
    let state = ensure_repo_is_ready(&repo, "clone", false, true)?;
    ensure_has_head_commit(&repo)?;

    let subdir = match args.subdir {
        Some(dir) => subdir::normalize_subdir(&dir)?,
        None => subdir::guess_subdir_from_remote(&args.remote)?,
    };

    let gitrepo_file = state.workdir.join(&subdir).join(".gitrepo");
    let is_reclone = args.force && gitrepo_file.is_file();

    if !is_reclone {
        assert_subdir_empty(&state, &subdir)?;
    }

    let subref = subdir::encode_subdir(&subdir)?;
    let refs = SubrepoRefs::new(&subref);

    let branch = match args.branch {
        Some(b) => b,
        None => git_cli::detect_remote_head_branch(&state.workdir, &args.remote)?,
    };

    let upstream_head_commit = remote::fetch_upstream_commit(&repo, &args.remote, &branch, &refs)?;

    if is_reclone {
        let bytes = std::fs::read(&gitrepo_file)?;
        let existing = GitRepoState::parse(&bytes)?;
        if !existing.commit.is_empty() && existing.commit == upstream_head_commit.to_string() {
            return Ok(format!("Subrepo '{subdir}' is up to date."));
        }
    }

    let upstream_tree_id = repo
        .find_commit(upstream_head_commit)
        .into_subrepo_result()?
        .tree_id()
        .into_subrepo_result()?
        .detach();

    let upstream_head_str = upstream_head_commit.to_string();

    let gitrepo_state = GitRepoState {
        remote: args.remote.clone(),
        branch: branch.clone(),
        commit: upstream_head_str.clone(),
        parent: state
            .head_commit
            .expect("head commit is required")
            .to_string(),
        method: args.method,
        cmdver: VERSION.to_string(),
    };

    let gitrepo_content = gitrepo_state.format();

    apply_tree_into_subdir(
        &repo,
        &state.workdir,
        &subdir,
        upstream_tree_id,
        &gitrepo_content,
        args.force,
        args.force,
    )?;

    repo.reference(
        refs.refs_commit.as_str(),
        upstream_head_commit,
        gix::refs::transaction::PreviousValue::Any,
        "",
    )
    .into_subrepo_result()?;

    let msg = match args.message {
        Some(m) => m,
        None => build_default_commit_message(
            &repo,
            "clone",
            &[args.remote.clone(), subdir.clone()],
            &subdir,
            &gitrepo_state.remote,
            &gitrepo_state.branch,
            Some(upstream_head_commit),
            Some(upstream_head_commit),
        )?,
    };

    git_commit_then_update_sync_ref(
        &state.workdir,
        &repo,
        &refs,
        CommitMessageSpec {
            message: Some(msg),
            message_file: args.message_file,
            edit: args.edit,
        },
    )?;

    Ok(format!(
        "Subrepo '{}' ({}) cloned into '{}'.",
        args.remote, branch, subdir
    ))
}

pub struct InitArgs {
    pub subdir: String,
    pub remote: Option<String>,
    pub branch: Option<String>,
    pub method: JoinMethod,
}

pub fn init(args: InitArgs) -> Result<String> {
    let repo = git_subrepo_core::repo::discover_repo()?;
    let state = ensure_repo_is_ready(&repo, "init", true, true)?;

    let subdir = subdir::normalize_subdir(&args.subdir)?;
    assert_subdir_ready_for_init(&repo, &subdir)?;

    let remote = args.remote.unwrap_or_else(|| "none".to_string());
    let branch = match args.branch {
        Some(b) => b,
        None => git_cli::init_default_branch(&repo)?,
    };

    let gitrepo_state = GitRepoState {
        remote: remote.clone(),
        branch: branch.clone(),
        commit: "".to_string(),
        parent: "".to_string(),
        method: args.method,
        cmdver: VERSION.to_string(),
    };
    let gitrepo_blob = repo
        .write_blob(gitrepo_state.format())
        .into_subrepo_result()?
        .detach();

    let head_commit = repo.head_commit().into_subrepo_result()?;
    let head_tree_id = head_commit.tree_id().into_subrepo_result()?.detach();

    let mut editor = repo.edit_tree(head_tree_id).into_subrepo_result()?;
    {
        let mut cursor = editor.cursor_at(&subdir).into_subrepo_result()?;
        cursor
            .upsert(".gitrepo", gix::object::tree::EntryKind::Blob, gitrepo_blob)
            .into_subrepo_result()?;
    }

    let new_tree_id = editor.write().into_subrepo_result()?;

    let commit_message = format!("git subrepo init {subdir}\n\n");
    repo.commit(
        "HEAD",
        commit_message,
        new_tree_id.detach(),
        [state.head_commit.expect("head commit is required")],
    )
    .into_subrepo_result()?;

    git_cli::run_or_command_failed(&state.workdir, &["reset", "--hard"])?;

    if remote == "none" {
        Ok(format!(
            "Subrepo created from '{}' (with no remote).",
            subdir
        ))
    } else {
        Ok(format!(
            "Subrepo created from '{}' with remote '{}' ({}).",
            subdir, remote, branch
        ))
    }
}

pub struct FetchArgs {
    pub subdir: String,
    pub remote: Option<String>,
    pub branch: Option<String>,
    pub force: bool,
}

pub fn fetch(args: FetchArgs) -> Result<String> {
    let repo = git_subrepo_core::repo::discover_repo()?;
    ensure_repo_is_ready(&repo, "fetch", true, false)?;

    let subdir = subdir::normalize_subdir(&args.subdir)?;
    let subref = subdir::encode_subdir(&subdir)?;
    let refs = SubrepoRefs::new(&subref);

    let mut state = read_gitrepo_state(&repo, &subdir)?;

    if let Some(remote) = args.remote {
        state.remote = remote;
    }
    if let Some(branch) = args.branch {
        state.branch = branch;
    }

    if state.remote == "none" {
        return Ok(format!("Ignored '{subdir}', no remote."));
    }

    remote::fetch_upstream_commit(&repo, &state.remote, &state.branch, &refs)?;

    Ok(format!(
        "Fetched '{subdir}' from '{}' ({}).",
        state.remote, state.branch
    ))
}

pub struct StatusArgs {
    pub subdir: Option<String>,
    pub all: bool,
    pub all_all: bool,
    pub fetch: bool,
}

pub fn status(args: StatusArgs) -> Result<String> {
    let repo = git_subrepo_core::repo::discover_repo()?;
    ensure_repo_is_ready(&repo, "status", true, false)?;

    let mut subrepos = list_subrepos_from_head(&repo)?;
    if let Some(subdir) = args.subdir.as_deref() {
        let normalized = subdir::normalize_subdir(subdir)?;
        subrepos.retain(|s| s == &normalized);
    } else if args.all_all {
        // Keep nested subrepos.
    } else if args.all {
        subrepos = filter_top_level_subrepos(subrepos);
    } else {
        return Err(Error::user(
            "Command 'status' requires a subdir unless '--all' or '--ALL' is provided.",
        ));
    }

    subrepos.sort();

    if args.fetch {
        for subdir in subrepos.iter() {
            let _ = fetch(FetchArgs {
                subdir: subdir.clone(),
                remote: None,
                branch: None,
                force: true,
            });
        }
    }

    let mut out = String::new();
    out.push_str(&format!("{} subrepos:\n", subrepos.len()));
    for subdir in subrepos {
        out.push_str(&format!("Git subrepo '{subdir}':\n"));
    }

    Ok(out.trim_end().to_string())
}

pub fn subrepos(include_nested: bool) -> Result<Vec<String>> {
    let repo = git_subrepo_core::repo::discover_repo()?;
    ensure_repo_is_ready(&repo, "status", true, false)?;

    let mut subrepos = list_subrepos_from_head(&repo)?;
    if !include_nested {
        subrepos = filter_top_level_subrepos(subrepos);
    }
    subrepos.sort();
    Ok(subrepos)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchesStyle {
    Oneline,
    Decorate,
    Stat,
    NameStatus,
}

pub struct PatchesArgs {
    pub subdir: Option<String>,

    pub all: bool,
    pub all_all: bool,

    pub since: Option<String>,
    pub from_ref: Option<String>,
    pub since_sync: bool,

    pub update_ref: bool,
    pub ref_name: Option<String>,

    pub style: PatchesStyle,
    pub reverse: bool,
}

pub fn patches(args: PatchesArgs) -> Result<String> {
    let repo = git_subrepo_core::repo::discover_repo()?;
    let state = ensure_repo_is_ready(&repo, "patches", true, false)?;

    if args.update_ref {
        let Some(subdir) = args.subdir.as_deref() else {
            return Err(Error::user(
                "Command 'patches' requires a subdir when using '--update-ref'.",
            ));
        };
        if args.all || args.all_all {
            return Err(Error::user(
                "Command 'patches' does not support '--all/--ALL' with '--update-ref'.",
            ));
        }

        let subdir = subdir::normalize_subdir(subdir)?;
        let subref = subdir::encode_subdir(&subdir)?;
        let refs = SubrepoRefs::new(&subref);

        let ref_name = args.ref_name.unwrap_or_else(|| refs.refs_sync.clone());
        let head = git_rev_parse_commit(&state.workdir, "HEAD")?;
        repo.reference(
            ref_name.as_str(),
            head,
            gix::refs::transaction::PreviousValue::Any,
            "",
        )
        .into_subrepo_result()?;

        return Ok(format!("Updated ref '{ref_name}' to {head}."));
    }

    if (args.since.is_some() as u8) + (args.from_ref.is_some() as u8) + (args.since_sync as u8) > 1
    {
        return Err(Error::user(
            "Options '--since', '--from-ref', and '--since-sync' are mutually exclusive.",
        ));
    }

    let subrepos = if let Some(subdir) = args.subdir.as_deref() {
        vec![subdir::normalize_subdir(subdir)?]
    } else if args.all_all {
        let mut subs = list_subrepos_from_head(&repo)?;
        subs.sort();
        subs
    } else if args.all {
        let subs = list_subrepos_from_head(&repo)?;
        let mut subs = filter_top_level_subrepos(subs);
        subs.sort();
        subs
    } else {
        return Err(Error::user(
            "Command 'patches' requires a subdir unless '--all' or '--ALL' is provided.",
        ));
    };

    let mut out = String::new();
    for subdir in subrepos {
        let is_tracked = state.workdir.join(&subdir).join(".gitrepo").is_file();
        if !is_tracked {
            out.push_str(&format!("warning: '{subdir}' is not a tracked subrepo\n"));
        }

        let subref = subdir::encode_subdir(&subdir)?;
        let refs = SubrepoRefs::new(&subref);

        let base = resolve_patches_base(&state.workdir, &repo, &subdir, &refs, &args)?;
        let range = format!("{}..HEAD", base);

        let lines = git_log_subdir(&state.workdir, &subdir, &range, args.style, args.reverse)?;

        out.push_str(&format!("Git subrepo '{subdir}':\n"));
        if lines.trim().is_empty() {
            out.push_str("  (no local patches since last sync)\n\n");
        } else {
            for line in lines.lines() {
                out.push_str("  ");
                out.push_str(line);
                out.push('\n');
            }
            out.push('\n');
        }
    }

    Ok(out.trim_end().to_string())
}

fn git_log_subdir(
    repo_root: &Path,
    subdir: &str,
    range: &str,
    style: PatchesStyle,
    reverse: bool,
) -> Result<String> {
    let mut argv: Vec<&str> = vec!["log", "--no-color"];
    if reverse {
        argv.push("--reverse");
    }

    match style {
        PatchesStyle::Oneline => argv.push("--oneline"),
        PatchesStyle::Decorate => {
            argv.push("--oneline");
            argv.push("--decorate");
        }
        PatchesStyle::Stat => argv.push("--stat"),
        PatchesStyle::NameStatus => argv.push("--name-status"),
    }

    argv.push(range);
    argv.push("--");
    argv.push(subdir);

    let out = git_cli::run_or_command_failed(repo_root, &argv)?;
    Ok(out.stdout.trim_end().to_string())
}

fn resolve_patches_base(
    repo_root: &Path,
    repo: &gix::Repository,
    subdir: &str,
    refs: &SubrepoRefs,
    args: &PatchesArgs,
) -> Result<gix::ObjectId> {
    if let Some(rev) = args.since.as_deref() {
        return git_rev_parse_commit(repo_root, rev);
    }

    if let Some(r) = args.from_ref.as_deref() {
        return git_rev_parse_commit(repo_root, r);
    }

    if !args.since_sync {
        if let Some(mut r) = repo
            .try_find_reference(refs.refs_sync.as_str())
            .into_subrepo_result()?
        {
            return Ok(r.peel_to_commit().into_subrepo_result()?.id);
        }
    }

    if let Some(base) = find_last_sync_anchor_commit(repo_root, subdir)? {
        return Ok(base);
    }

    Err(Error::user(format!(
        "Cannot determine sync base for '{subdir}'.\n\
Run 'git subrepo patches {subdir} --since <rev>' or\n\
  'git subrepo patches {subdir} --update-ref' (updates refs/subrepo/<subref>/sync)."
    )))
}

fn find_last_sync_anchor_commit(repo_root: &Path, subdir: &str) -> Result<Option<gix::ObjectId>> {
    let fmt = "--format=%H%x00%B%x00";
    let out = git_cli::run_or_command_failed(repo_root, &["log", "--no-color", fmt, "--", subdir])?;

    let mut it = out.stdout.split('\0');
    loop {
        let Some(hash) = it.next() else { break };
        if hash.is_empty() {
            break;
        }

        let Some(message) = it.next() else { break };
        if is_sync_anchor_for_subdir(message, subdir) {
            return Ok(Some(parse_object_id(hash)?));
        }
    }

    Ok(None)
}

fn is_sync_anchor_for_subdir(message: &str, subdir: &str) -> bool {
    let Some(subject) = message.lines().next() else {
        return false;
    };

    let Some(rest) = subject.strip_prefix("git subrepo ") else {
        return false;
    };

    let cmd = rest.split_whitespace().next().unwrap_or("");
    if !matches!(cmd, "clone" | "pull" | "commit" | "push") {
        return false;
    }

    match parse_subdir_from_commit_message(message) {
        Some(s) => s == subdir,
        None => false,
    }
}

fn parse_subdir_from_commit_message(message: &str) -> Option<String> {
    for line in message.lines() {
        let Some(rest) = line.strip_prefix("  subdir:") else {
            continue;
        };

        let raw = rest.trim();
        let raw = raw.strip_prefix('"')?.strip_suffix('"')?;
        return Some(raw.to_string());
    }

    None
}

#[cfg(test)]
mod patches_message_tests {
    use super::{is_sync_anchor_for_subdir, parse_subdir_from_commit_message};

    #[test]
    fn parse_subdir_from_commit_message_extracts_quoted_subdir() {
        let msg = "git subrepo pull foo\n\nsubrepo:\n  subdir:   \"vendor/foo\"\n";
        assert_eq!(
            parse_subdir_from_commit_message(msg),
            Some("vendor/foo".to_string())
        );
    }

    #[test]
    fn is_sync_anchor_for_subdir_matches_supported_commands() {
        let msg = "git subrepo pull (merge) foo\n\nsubrepo:\n  subdir:   \"foo\"\n";
        assert!(is_sync_anchor_for_subdir(msg, "foo"));
        assert!(!is_sync_anchor_for_subdir(msg, "bar"));
    }

    #[test]
    fn is_sync_anchor_for_subdir_rejects_unknown_commands() {
        let msg = "git subrepo unknown foo\n\nsubrepo:\n  subdir:   \"foo\"\n";
        assert!(!is_sync_anchor_for_subdir(msg, "foo"));
    }
}

pub struct CleanArgs {
    pub subdir: String,
    pub force: bool,
}

pub fn clean(args: CleanArgs) -> Result<Vec<String>> {
    let repo = git_subrepo_core::repo::discover_repo()?;
    let state = ensure_repo_is_ready(&repo, "clean", true, false)?;

    let subdir = subdir::normalize_subdir(&args.subdir)?;
    let subref = subdir::encode_subdir(&subdir)?;
    let refs = SubrepoRefs::new(&subref);

    remove_worktree(&state.workdir, &refs.branch, "clean")?;

    let mut removed = Vec::new();

    if delete_ref_if_exists(&repo, &format!("refs/heads/{}", refs.branch))? {
        removed.push(format!("Removed branch '{}'.", refs.branch));
    }

    if args.force {
        for name in [
            refs.refs_fetch.as_str(),
            refs.refs_branch.as_str(),
            refs.refs_commit.as_str(),
            refs.refs_push.as_str(),
            refs.refs_sync.as_str(),
            &format!("refs/original/refs/heads/{}", refs.branch),
        ] {
            let _ = delete_ref_if_exists(&repo, name)?;
        }
    }

    Ok(removed)
}

pub struct ConfigArgs {
    pub subdir: String,
    pub option: String,
    pub value: Option<String>,
    pub force: bool,
}

pub fn config(args: ConfigArgs) -> Result<String> {
    let repo = git_subrepo_core::repo::discover_repo()?;
    ensure_repo_is_ready(&repo, "config", true, false)?;

    let subdir = subdir::normalize_subdir(&args.subdir)?;
    let option = args.option;

    if !matches!(
        option.as_str(),
        "branch" | "cmdver" | "commit" | "method" | "remote" | "version" | "parent"
    ) {
        return Err(Error::user(format!("Option {option} not recognized")));
    }

    let gitrepo_path = gitrepo_path(&repo, &subdir)?;
    let bytes = std::fs::read(&gitrepo_path)?;
    let mut state = GitRepoState::parse(&bytes)?;

    match args.value {
        None => {
            let value = match option.as_str() {
                "remote" => state.remote.clone(),
                "branch" => state.branch.clone(),
                "commit" => state.commit.clone(),
                "parent" => state.parent.clone(),
                "method" => state.method.as_str().to_string(),
                "cmdver" | "version" => state.cmdver.clone(),
                _ => unreachable!(),
            };
            Ok(format!(
                "Subrepo '{subdir}' option '{option}' has value '{value}'."
            ))
        }
        Some(value) => {
            if !args.force && option != "method" {
                return Err(Error::user(
                    "This option is autogenerated, use '--force' to override.",
                ));
            }

            if option == "method" && value != "merge" && value != "rebase" {
                return Err(Error::user(
                    "Not a valid method. Valid options are 'merge' or 'rebase'.",
                ));
            }

            match option.as_str() {
                "remote" => state.remote = value.clone(),
                "branch" => state.branch = value.clone(),
                "commit" => state.commit = value.clone(),
                "parent" => state.parent = value.clone(),
                "method" => state.method = value.parse()?,
                "cmdver" | "version" => state.cmdver = value.clone(),
                _ => unreachable!(),
            }

            std::fs::write(&gitrepo_path, state.format())?;
            Ok(format!(
                "Subrepo '{subdir}' option '{option}' set to '{value}'."
            ))
        }
    }
}

pub struct BranchArgs {
    pub subdir: String,
    pub force: bool,
    pub fetch: bool,
}

pub fn branch(args: BranchArgs) -> Result<String> {
    let repo = git_subrepo_core::repo::discover_repo()?;
    let state = ensure_repo_is_ready(&repo, "branch", true, true)?;

    let subdir = subdir::normalize_subdir(&args.subdir)?;
    let subref = subdir::encode_subdir(&subdir)?;
    let refs = SubrepoRefs::new(&subref);

    if args.fetch {
        let _ = fetch(FetchArgs {
            subdir: subdir.clone(),
            remote: None,
            branch: None,
            force: true,
        });
    }

    if args.force {
        delete_branch_and_worktree(&repo, &state, &refs.branch, "branch")?;
    }

    if repo
        .try_find_reference(format!("refs/heads/{}", refs.branch).as_str())
        .into_subrepo_result()?
        .is_some()
    {
        return Err(Error::user(format!(
            "Branch '{}' already exists. Use '--force' to override.",
            refs.branch
        )));
    }

    let gitrepo = read_gitrepo_state(&repo, &subdir)?;

    let created =
        create_subrepo_branch_and_worktree(&repo, &state, &subdir, &refs, &gitrepo, None)?;

    Ok(format!(
        "Created branch '{}' and worktree '{}'.",
        refs.branch, created.worktree_display
    ))
}

pub struct PullArgs {
    pub subdir: String,
    pub force: bool,
    pub remote: Option<String>,
    pub branch: Option<String>,
    pub update: bool,
    pub message: Option<String>,
    pub message_file: Option<String>,
    pub edit: bool,
}

pub fn pull(args: PullArgs) -> Result<String> {
    let repo = git_subrepo_core::repo::discover_repo()?;
    let state = ensure_repo_is_ready(&repo, "pull", true, true)?;

    let subdir = subdir::normalize_subdir(&args.subdir)?;
    let subref = subdir::encode_subdir(&subdir)?;
    let refs = SubrepoRefs::new(&subref);

    let mut gitrepo = read_gitrepo_state(&repo, &subdir)?;

    let override_remote = args.remote.is_some();
    let override_branch = args.branch.is_some();

    if args.update && !(override_remote || override_branch) {
        return Err(Error::user(
            "Can't use '--update' without '--branch' or '--remote'.",
        ));
    }

    if let Some(remote) = args.remote {
        gitrepo.remote = remote;
    }
    if let Some(branch) = args.branch {
        gitrepo.branch = branch;
    }

    if gitrepo.remote == "none" {
        return Err(Error::user(format!(
            "Can't fetch subrepo. Remote is 'none' in '{subdir}/.gitrepo'."
        )));
    }

    let upstream_head =
        remote::fetch_upstream_commit(&repo, &gitrepo.remote, &gitrepo.branch, &refs)?;

    if !args.force && !gitrepo.commit.is_empty() {
        let current = parse_object_id(&gitrepo.commit)?;
        if upstream_head == current && !args.update {
            return Ok(format!("Subrepo '{subdir}' is up to date."));
        }
    }

    // `--force` for pull behaves like `clone --force` upstream, but keeps the `pull` UX.
    if args.force {
        let upstream_head_str = upstream_head.to_string();

        let msg = match args.message {
            Some(m) => m,
            None => build_default_commit_message(
                &repo,
                "pull",
                &[subdir.clone(), "--force".to_string()],
                &subdir,
                &gitrepo.remote,
                &gitrepo.branch,
                Some(upstream_head),
                Some(upstream_head),
            )?,
        };

        commit_subrepo_to_mainline(
            &repo,
            &state,
            &subdir,
            &refs,
            &mut gitrepo,
            upstream_head,
            upstream_head_str.as_str(),
            CommitOptions {
                command: "pull",
                message: Some(msg),
                message_file: args.message_file,
                edit: args.edit,
                force: true,
            },
        )?;

        return Ok(format!(
            "Subrepo '{subdir}' pulled from '{}' ({}).",
            gitrepo.remote, gitrepo.branch
        ));
    }

    delete_branch_and_worktree(&repo, &state, &refs.branch, "pull")?;

    let created =
        create_subrepo_branch_and_worktree(&repo, &state, &subdir, &refs, &gitrepo, None)?;

    let merge_res = match gitrepo.method {
        JoinMethod::Rebase => git_cli::run_raw(
            &created.worktree_abs,
            &["rebase", refs.refs_fetch.as_str(), refs.branch.as_str()],
        )?,
        JoinMethod::Merge => {
            git_cli::run_raw(&created.worktree_abs, &["merge", refs.refs_fetch.as_str()])?
        }
    };

    if !merge_res.status.success() {
        return Err(Error::user("The \"git merge\" command failed:"));
    }

    let branch_tip = git_rev_parse_commit(&state.workdir, refs.branch.as_str())?;
    repo.reference(
        refs.refs_branch.as_str(),
        branch_tip,
        gix::refs::transaction::PreviousValue::Any,
        "",
    )
    .into_subrepo_result()?;

    commit_subrepo_to_mainline(
        &repo,
        &state,
        &subdir,
        &refs,
        &mut gitrepo,
        upstream_head,
        refs.branch.as_str(),
        CommitOptions {
            command: "pull",
            message: args.message,
            message_file: args.message_file,
            edit: args.edit,
            force: false,
        },
    )?;

    Ok(format!(
        "Subrepo '{subdir}' pulled from '{}' ({}).",
        gitrepo.remote, gitrepo.branch
    ))
}

pub struct PushArgs {
    pub subdir: String,
    pub force: bool,
    pub squash: bool,
    pub remote: Option<String>,
    pub branch: Option<String>,
    pub update: bool,
    pub message: Option<String>,
    pub message_file: Option<String>,
}

pub fn push(args: PushArgs) -> Result<String> {
    let repo = git_subrepo_core::repo::discover_repo()?;
    let state = ensure_repo_is_ready(&repo, "push", true, true)?;

    let subdir = subdir::normalize_subdir(&args.subdir)?;
    let subref = subdir::encode_subdir(&subdir)?;
    let refs = SubrepoRefs::new(&subref);

    let mut gitrepo = read_gitrepo_state(&repo, &subdir)?;

    let override_remote = args.remote.is_some();
    let override_branch = args.branch.is_some();

    if args.update && !(override_remote || override_branch) {
        return Err(Error::user(
            "Can't use '--update' without '--branch' or '--remote'.",
        ));
    }

    if let Some(remote) = args.remote {
        gitrepo.remote = remote;
    }
    if let Some(branch) = args.branch {
        gitrepo.branch = branch;
    }

    if gitrepo.remote == "none" {
        return Err(Error::user(format!(
            "Can't fetch subrepo. Remote is 'none' in '{subdir}/.gitrepo'."
        )));
    }

    let upstream_head =
        remote::fetch_upstream_commit(&repo, &gitrepo.remote, &gitrepo.branch, &refs).ok();

    if let (Some(upstream_head), false) = (upstream_head, args.force) {
        if !gitrepo.commit.is_empty() {
            let current = parse_object_id(&gitrepo.commit)?;
            if upstream_head != current {
                return Err(Error::user(
                    "There are new changes upstream, you need to pull first.",
                ));
            }
        }
    }

    delete_branch_and_worktree(&repo, &state, &refs.branch, "push")?;

    let parent_override = if args.squash {
        Some(git_rev_parse_commit(&state.workdir, "HEAD^")?)
    } else {
        None
    };

    let created = create_subrepo_branch_and_worktree(
        &repo,
        &state,
        &subdir,
        &refs,
        &gitrepo,
        parent_override,
    )?;

    if matches!(gitrepo.method, JoinMethod::Rebase) {
        if upstream_head.is_some() {
            let res = git_cli::run_raw(
                &created.worktree_abs,
                &["rebase", refs.refs_fetch.as_str(), refs.branch.as_str()],
            )?;
            if !res.status.success() {
                return Err(Error::user("The \"git rebase\" command failed:"));
            }
        }
    }

    let tip = git_rev_parse_commit(&state.workdir, refs.branch.as_str())?;

    if let Some(upstream_head) = upstream_head {
        if tip == upstream_head {
            delete_branch_and_worktree(&repo, &state, &refs.branch, "push")?;
            return Ok(format!("Subrepo '{subdir}' has no new commits to push."));
        }

        if !args.force {
            ensure_contains_commit(&repo, upstream_head, tip)?;
        }
    }

    let mut push_args = vec!["push".to_string()];
    if args.force {
        push_args.push("--force".to_string());
    }
    push_args.push(gitrepo.remote.clone());
    push_args.push(format!("{}:{}", refs.branch, gitrepo.branch));

    let push_args_ref: Vec<&str> = push_args.iter().map(|s| s.as_str()).collect();
    git_cli::run_or_command_failed(&state.workdir, &push_args_ref)?;

    repo.reference(
        refs.refs_push.as_str(),
        tip,
        gix::refs::transaction::PreviousValue::Any,
        "",
    )
    .into_subrepo_result()?;

    delete_branch_and_worktree(&repo, &state, &refs.branch, "push")?;

    gitrepo.commit = tip.to_string();
    gitrepo.parent = state
        .head_commit
        .expect("head commit is required")
        .to_string();
    gitrepo.cmdver = VERSION.to_string();

    write_gitrepo_state(&state, &subdir, &gitrepo)?;
    git_cli::run_or_command_failed(
        &state.workdir,
        &["add", "-f", "--", &format!("{subdir}/.gitrepo")],
    )?;

    let msg = match args.message {
        Some(m) => m,
        None => build_default_commit_message(
            &repo,
            "push",
            &[subdir.clone()],
            &subdir,
            &gitrepo.remote,
            &gitrepo.branch,
            Some(tip),
            Some(tip),
        )?,
    };

    git_commit_then_update_sync_ref(
        &state.workdir,
        &repo,
        &refs,
        CommitMessageSpec {
            message: Some(msg),
            message_file: args.message_file,
            edit: false,
        },
    )?;

    Ok(format!(
        "Subrepo '{subdir}' pushed to '{}' ({}).",
        gitrepo.remote, gitrepo.branch
    ))
}

pub struct CommitArgs {
    pub subdir: String,
    pub commit_ref: Option<String>,
    pub force: bool,
    pub fetch: bool,
    pub message: Option<String>,
    pub message_file: Option<String>,
    pub edit: bool,
}

pub fn commit(args: CommitArgs) -> Result<String> {
    let repo = git_subrepo_core::repo::discover_repo()?;
    let state = ensure_repo_is_ready(&repo, "commit", true, true)?;

    let subdir = subdir::normalize_subdir(&args.subdir)?;
    let subref = subdir::encode_subdir(&subdir)?;
    let refs = SubrepoRefs::new(&subref);

    if args.fetch {
        let _ = fetch(FetchArgs {
            subdir: subdir.clone(),
            remote: None,
            branch: None,
            force: true,
        });
    }

    let mut gitrepo = read_gitrepo_state(&repo, &subdir)?;

    let upstream_head = match repo
        .try_find_reference(refs.refs_fetch.as_str())
        .into_subrepo_result()?
    {
        Some(mut r) => r.peel_to_commit().into_subrepo_result()?.id,
        None => {
            return Err(Error::user(format!(
                "Can't find ref '{}'. Try using -F.",
                refs.refs_fetch
            )))
        }
    };

    let subrepo_commit_ref = args.commit_ref.unwrap_or_else(|| refs.branch.clone());

    commit_subrepo_to_mainline(
        &repo,
        &state,
        &subdir,
        &refs,
        &mut gitrepo,
        upstream_head,
        &subrepo_commit_ref,
        CommitOptions {
            command: "commit",
            message: args.message,
            message_file: args.message_file,
            edit: args.edit,
            force: args.force,
        },
    )?;

    Ok(format!(
        "Subrepo commit '{}' committed as\nsubdir '{}/' to branch '{}'.",
        subrepo_commit_ref, subdir, state.head_branch
    ))
}

fn ensure_has_head_commit(repo: &gix::Repository) -> Result<()> {
    match repo.head_commit() {
        Ok(_) => Ok(()),
        Err(_) => Err(Error::user("You can't clone into an empty repository")),
    }
}

fn assert_subdir_empty(state: &RepoState, subdir: &str) -> Result<()> {
    let path = state.workdir.join(subdir);
    if !path.exists() {
        return Ok(());
    }
    if !path.is_dir() {
        return Err(Error::user(format!(
            "The subdir '{subdir}' exists and is not empty."
        )));
    }

    if std::fs::read_dir(&path)?.next().is_some() {
        return Err(Error::user(format!(
            "The subdir '{subdir}' exists and is not empty."
        )));
    }
    Ok(())
}

fn assert_subdir_ready_for_init(repo: &gix::Repository, subdir: &str) -> Result<()> {
    let workdir = repo
        .workdir()
        .ok_or_else(|| Error::user("Can't 'subrepo init' outside a working tree."))?;

    let path = workdir.join(subdir);
    if !path.exists() {
        return Err(Error::user(format!(
            "The subdir '{subdir}' does not exist."
        )));
    }

    if path.join(".gitrepo").exists() {
        return Err(Error::user(format!(
            "The subdir '{subdir}' is already a subrepo."
        )));
    }

    let head_tree = repo.head_tree_id_or_empty().into_subrepo_result()?.detach();

    let head_tree = repo.find_tree(head_tree).into_subrepo_result()?;

    let entry = head_tree
        .lookup_entry_by_path(Path::new(subdir))
        .into_subrepo_result()?;

    let Some(entry) = entry else {
        return Err(Error::user(format!(
            "The subdir '{subdir}' is not part of this repo."
        )));
    };

    if !entry.mode().is_tree() {
        return Err(Error::user(format!(
            "The subdir '{subdir}' is not part of this repo."
        )));
    }

    Ok(())
}

fn list_subrepos_from_head(repo: &gix::Repository) -> Result<Vec<String>> {
    let head_tree_id = repo.head_tree_id_or_empty().into_subrepo_result()?.detach();
    let head_tree = repo.find_tree(head_tree_id).into_subrepo_result()?;

    let entries = head_tree
        .traverse()
        .breadthfirst
        .files()
        .into_subrepo_result()?;

    let mut out = Vec::new();

    for e in entries {
        if !e.filepath.ends_with(b"/.gitrepo") {
            continue;
        }
        let subdir = e
            .filepath
            .strip_suffix(b"/.gitrepo")
            .expect("checked above");
        out.push(subdir.as_bstr().to_string());
    }

    Ok(out)
}

fn filter_top_level_subrepos(mut subrepos: Vec<String>) -> Vec<String> {
    subrepos.sort();
    let mut out: Vec<String> = Vec::new();
    'outer: for subdir in subrepos {
        for parent in &out {
            if subdir.starts_with(parent) && subdir.as_bytes().get(parent.len()) == Some(&b'/') {
                continue 'outer;
            }
        }
        out.push(subdir);
    }
    out
}

fn gitrepo_path(repo: &gix::Repository, subdir: &str) -> Result<PathBuf> {
    let workdir = repo
        .workdir()
        .ok_or_else(|| Error::user("Can't 'subrepo' outside a working tree."))?;
    Ok(workdir.join(subdir).join(".gitrepo"))
}

fn read_gitrepo_state(repo: &gix::Repository, subdir: &str) -> Result<GitRepoState> {
    let path = gitrepo_path(repo, subdir)?;
    match std::fs::read(path) {
        Ok(bytes) => GitRepoState::parse(&bytes),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            Err(Error::user(format!("No '{subdir}/.gitrepo' file.")))
        }
        Err(err) => Err(err.into()),
    }
}

fn write_gitrepo_state(state: &RepoState, subdir: &str, gitrepo: &GitRepoState) -> Result<()> {
    let path = state.workdir.join(subdir).join(".gitrepo");
    std::fs::write(path, gitrepo.format())?;
    Ok(())
}

fn delete_ref_if_exists(repo: &gix::Repository, name: &str) -> Result<bool> {
    use gix::refs::transaction::{Change, PreviousValue, RefEdit, RefLog};

    let Some(reference) = repo.try_find_reference(name).into_subrepo_result()? else {
        return Ok(false);
    };

    let full_name = reference.name().to_owned();
    repo.edit_reference(RefEdit {
        change: Change::Delete {
            expected: PreviousValue::Any,
            log: RefLog::AndReference,
        },
        name: full_name,
        deref: false,
    })
    .into_subrepo_result()?;

    Ok(true)
}

fn delete_branch_and_worktree(
    repo: &gix::Repository,
    state: &RepoState,
    branch: &str,
    command: &str,
) -> Result<()> {
    remove_worktree(&state.workdir, branch, command)?;
    let _ = delete_ref_if_exists(repo, &format!("refs/heads/{branch}"))?;
    Ok(())
}

fn assert_worktree_is_clean(worktree: &Path, command: &str) -> Result<()> {
    let pwd = worktree.display();

    // Keep behavior aligned with upstream: don't consider untracked files.
    let _ = git_cli::run_or_command_failed(
        worktree,
        &["update-index", "-q", "--ignore-submodules", "--refresh"],
    )?;

    let diff = git_cli::run_raw(worktree, &["diff-files", "--quiet", "--ignore-submodules"])?;
    if !diff.status.success() {
        return Err(Error::user(format!(
            "Can't {command} subrepo. Unstaged changes. ({pwd})"
        )));
    }

    let diff = git_cli::run_raw(
        worktree,
        &["diff-index", "--quiet", "--ignore-submodules", "HEAD"],
    )?;
    if !diff.status.success() {
        return Err(Error::user(format!(
            "Can't {command} subrepo. Working tree has changes. ({pwd})"
        )));
    }

    let diff = git_cli::run_raw(
        worktree,
        &[
            "diff-index",
            "--quiet",
            "--cached",
            "--ignore-submodules",
            "HEAD",
        ],
    )?;
    if !diff.status.success() {
        return Err(Error::user(format!(
            "Can't {command} subrepo. Index has changes. ({pwd})"
        )));
    }

    Ok(())
}

fn remove_worktree(repo_root: &Path, branch: &str, command: &str) -> Result<()> {
    let paths = subrepo_worktree_paths(repo_root, branch)?;
    if paths.worktree_abs.exists() {
        assert_worktree_is_clean(&paths.worktree_abs, command)?;
        std::fs::remove_dir_all(&paths.worktree_abs)?;
    }

    // Prune even if we didn't delete the directory, as the metadata might still exist.
    let _ = git_cli::run_raw(repo_root, &["worktree", "prune"])?;
    Ok(())
}

struct WorktreePaths {
    worktree_abs: PathBuf,
    worktree_display: String,
}

fn subrepo_worktree_paths(repo_root: &Path, branch: &str) -> Result<WorktreePaths> {
    let common_dir_str = git_cli::git_common_dir_str(repo_root)?;
    let common_dir_path = git_cli::git_common_dir_path(repo_root)?;

    let worktree_display = PathBuf::from(common_dir_str)
        .join("tmp")
        .join(branch)
        .to_string_lossy()
        .to_string();
    let worktree_abs = common_dir_path.join("tmp").join(branch);

    Ok(WorktreePaths {
        worktree_abs,
        worktree_display,
    })
}

struct CreatedBranch {
    worktree_abs: PathBuf,
    worktree_display: String,
}

fn create_subrepo_branch_and_worktree(
    repo: &gix::Repository,
    state: &RepoState,
    subdir: &str,
    refs: &SubrepoRefs,
    gitrepo: &GitRepoState,
    parent_override: Option<gix::ObjectId>,
) -> Result<CreatedBranch> {
    let branch = refs.branch.as_str();

    let worktree_paths = subrepo_worktree_paths(&state.workdir, branch)?;

    if let Some(parent_override) = parent_override {
        create_subrepo_branch_with_parent(repo, state, subdir, refs, gitrepo, parent_override)?;
    } else if gitrepo.parent.is_empty() {
        create_subrepo_branch_without_parent(repo, state, subdir, branch)?;
    } else {
        let parent = parse_object_id(&gitrepo.parent)?;
        create_subrepo_branch_with_parent(repo, state, subdir, refs, gitrepo, parent)?;
    }

    let tip = git_rev_parse_commit(&state.workdir, branch)?;
    repo.reference(
        refs.refs_branch.as_str(),
        tip,
        gix::refs::transaction::PreviousValue::Any,
        "",
    )
    .into_subrepo_result()?;

    let worktree_abs = worktree_paths.worktree_abs.to_string_lossy().to_string();
    git_cli::run_or_command_failed(&state.workdir, &["worktree", "add", &worktree_abs, branch])?;

    Ok(CreatedBranch {
        worktree_abs: worktree_paths.worktree_abs,
        worktree_display: worktree_paths.worktree_display,
    })
}

fn create_subrepo_branch_without_parent(
    repo: &gix::Repository,
    state: &RepoState,
    subdir: &str,
    branch: &str,
) -> Result<()> {
    let head = state.head_commit.expect("head commit is required");

    let filtered_tip = filter_branch::subdirectory_filter(
        repo,
        head,
        None,
        subdir,
        filter_branch::Options::default(),
    )
    .map_err(|err| Error::user(err.to_string()))?;

    let rewritten_tip = filter_branch::tree_filter_remove_path_first_parent(
        repo,
        filtered_tip,
        None,
        ".gitrepo",
        filter_branch::Options::default(),
    )
    .map_err(|err| Error::user(err.to_string()))?;

    repo.reference(
        format!("refs/heads/{branch}"),
        rewritten_tip,
        gix::refs::transaction::PreviousValue::Any,
        "",
    )
    .into_subrepo_result()?;

    Ok(())
}

fn create_subrepo_branch_with_parent(
    repo: &gix::Repository,
    state: &RepoState,
    subdir: &str,
    refs: &SubrepoRefs,
    gitrepo: &GitRepoState,
    parent: gix::ObjectId,
) -> Result<()> {
    let head = state.head_commit.expect("head commit is required");
    ensure_ancestor(repo, parent, head, subdir, &gitrepo_path(repo, subdir)?)?;

    let rev_list_range = format!("{parent}..HEAD");
    let rev_list = git_cli::run_checked_stdout(
        &state.workdir,
        &[
            "rev-list",
            "--reverse",
            "--ancestry-path",
            "--topo-order",
            &rev_list_range,
        ],
    )?;

    let committer = repo
        .committer()
        .ok_or_else(|| Error::user("Committer not configured"))
        .and_then(|r| r.into_subrepo_result())?;

    let fetch_head = match repo
        .try_find_reference(refs.refs_fetch.as_str())
        .into_subrepo_result()?
    {
        Some(_) => Some(git_rev_parse_commit(
            &state.workdir,
            refs.refs_fetch.as_str(),
        )?),
        None => None,
    };

    let subref_for_message = refs
        .branch
        .strip_prefix("subrepo/")
        .unwrap_or(refs.branch.as_str())
        .to_string();

    let mut prev_new_commit: Option<gix::ObjectId> = None;
    let mut ancestor_main: Option<gix::ObjectId> = None;

    let mut first_upstream: Option<gix::ObjectId> = None;
    let mut last_upstream: Option<gix::ObjectId> = None;

    for line in rev_list.lines() {
        let commit_id = line.trim().parse::<gix::ObjectId>().into_subrepo_result()?;

        let commit = repo.find_commit(commit_id).into_subrepo_result()?;

        if let Some(ancestor) = ancestor_main {
            if !commit.parent_ids().any(|p| p.detach() == ancestor) {
                continue;
            }
        }

        let Some(upstream_commit) = gitrepo_commit_at(repo, commit_id, subdir)? else {
            continue;
        };

        if let Some(fetch_head) = fetch_head {
            let base = repo
                .merge_base(upstream_commit, fetch_head)
                .map(|id| id.detach())
                .ok();
            if base != Some(upstream_commit) {
                return Err(Error::user(format!(
                    "Local repository does not contain {upstream_commit}. Try to 'git subrepo fetch {subref_for_message}' or add the '-F' flag to always fetch the latest content.",
                )));
            }
        }

        ancestor_main = Some(commit_id);

        let mut parents: Vec<gix::ObjectId> = Vec::new();

        if let Some(prev) = prev_new_commit {
            parents.push(prev);
        }

        let mut second_parent = None;
        if first_upstream.is_none() {
            first_upstream = Some(upstream_commit);
            second_parent = Some(upstream_commit);
            if gitrepo.method != JoinMethod::Rebase {
                last_upstream = Some(upstream_commit);
            }
        } else if gitrepo.method != JoinMethod::Rebase {
            if last_upstream != Some(upstream_commit) {
                second_parent = Some(upstream_commit);
                last_upstream = Some(upstream_commit);
            }
        }

        if parents.is_empty() {
            if let Some(upstream) = second_parent {
                parents.push(upstream);
            }
        } else if let Some(upstream) = second_parent {
            parents.push(upstream);
        }

        let tree_id = subtree_tree_id_at_commit(repo, commit_id, subdir)?
            .unwrap_or_else(|| repo.empty_tree().id);

        let author_owned = commit
            .author()
            .into_subrepo_result()?
            .to_owned()
            .into_subrepo_result()?;
        let mut author_time = gix::date::parse::TimeBuf::default();
        let author = author_owned.to_ref(&mut author_time);

        let message = commit.message_raw_sloppy().to_str_lossy().into_owned();

        let new_commit = repo
            .new_commit_as(committer, author, message, tree_id, parents)
            .into_subrepo_result()?;

        prev_new_commit = Some(new_commit.id);
    }

    let tip = prev_new_commit.ok_or_else(|| Error::user("No commits found for subrepo branch"))?;

    repo.reference(
        format!("refs/heads/{}", refs.branch),
        tip,
        gix::refs::transaction::PreviousValue::Any,
        "",
    )
    .into_subrepo_result()?;

    let rewritten_tip = filter_branch::tree_filter_remove_path_first_parent(
        repo,
        tip,
        first_upstream,
        ".gitrepo",
        filter_branch::Options::default(),
    )
    .map_err(|err| Error::user(err.to_string()))?;

    repo.reference(
        format!("refs/heads/{}", refs.branch),
        rewritten_tip,
        gix::refs::transaction::PreviousValue::Any,
        "",
    )
    .into_subrepo_result()?;

    Ok(())
}

fn gitrepo_commit_at(
    repo: &gix::Repository,
    commit_id: gix::ObjectId,
    subdir: &str,
) -> Result<Option<gix::ObjectId>> {
    let Some(subtree_id) = subtree_tree_id_at_commit(repo, commit_id, subdir)? else {
        return Ok(None);
    };

    let tree = repo.find_tree(subtree_id).into_subrepo_result()?;

    let Some(entry) = tree.find_entry(b".gitrepo") else {
        return Ok(None);
    };
    if !entry.mode().is_blob() {
        return Ok(None);
    }

    let blob = repo.find_blob(entry.id()).into_subrepo_result()?;

    let state = GitRepoState::parse(&blob.data)?;
    if state.commit.is_empty() {
        return Ok(None);
    }

    Ok(Some(parse_object_id(&state.commit)?))
}

fn subtree_tree_id_at_commit(
    repo: &gix::Repository,
    commit_id: gix::ObjectId,
    subdir: &str,
) -> Result<Option<gix::ObjectId>> {
    let commit = repo.find_commit(commit_id).into_subrepo_result()?;
    let tree = commit.tree().into_subrepo_result()?;
    let entry = tree
        .lookup_entry_by_path(Path::new(subdir))
        .into_subrepo_result()?;
    let Some(entry) = entry else {
        return Ok(None);
    };
    if !entry.mode().is_tree() {
        return Ok(None);
    }
    Ok(Some(entry.object_id()))
}

fn git_rev_parse_commit(repo_root: &Path, spec: &str) -> Result<gix::ObjectId> {
    let peeled = format!("{spec}^0");
    let hex = git_cli::run_checked_stdout(repo_root, &["rev-parse", &peeled])?;
    hex.parse::<gix::ObjectId>().into_subrepo_result()
}

fn parse_object_id(hex: &str) -> Result<gix::ObjectId> {
    if hex.is_empty() {
        return Err(Error::user("Empty object id"));
    }
    hex.parse::<gix::ObjectId>().into_subrepo_result()
}

fn ensure_contains_commit(
    repo: &gix::Repository,
    ancestor: gix::ObjectId,
    head: gix::ObjectId,
) -> Result<()> {
    let base = repo
        .merge_base(ancestor, head)
        .into_subrepo_result()?
        .detach();
    if base != ancestor {
        return Err(Error::user("Can't commit: does not contain upstream HEAD."));
    }
    Ok(())
}

fn ensure_ancestor(
    repo: &gix::Repository,
    ancestor: gix::ObjectId,
    head: gix::ObjectId,
    subdir: &str,
    gitrepo_path: &Path,
) -> Result<()> {
    let base = repo
        .merge_base(ancestor, head)
        .into_subrepo_result()?
        .detach();
    if base != ancestor {
        let gitrepo_path_display = gitrepo_path.display();
        return Err(Error::user(format!(
            "The last sync point (where upstream and the subrepo were equal) is not an ancestor.\n\
This is usually caused by a rebase affecting that commit.\n\
To recover set the subrepo parent in '{gitrepo_path_display}'\n\
to '<previous-merge-point>'\n\
and validate the subrepo by comparing with 'git subrepo branch {subdir}'",
        )));
    }
    Ok(())
}

struct CommitOptions {
    command: &'static str,
    message: Option<String>,
    message_file: Option<String>,
    edit: bool,
    force: bool,
}

struct CommitMessageSpec {
    message: Option<String>,
    message_file: Option<String>,
    edit: bool,
}

fn commit_subrepo_to_mainline(
    repo: &gix::Repository,
    state: &RepoState,
    subdir: &str,
    refs: &SubrepoRefs,
    gitrepo: &mut GitRepoState,
    upstream_head: gix::ObjectId,
    subrepo_commit_ref: &str,
    opts: CommitOptions,
) -> Result<()> {
    let merged_commit = git_rev_parse_commit(&state.workdir, subrepo_commit_ref)
        .map_err(|_| Error::user(format!("Commit ref '{subrepo_commit_ref}' does not exist.")))?;

    if !opts.force {
        ensure_contains_commit(repo, upstream_head, merged_commit)?;
    }

    let merged_tree_id = repo
        .find_commit(merged_commit)
        .into_subrepo_result()?
        .tree_id()
        .into_subrepo_result()?
        .detach();

    gitrepo.commit = upstream_head.to_string();

    if upstream_head == merged_commit {
        gitrepo.parent = state
            .head_commit
            .expect("head commit is required")
            .to_string();
    }

    gitrepo.cmdver = VERSION.to_string();

    write_gitrepo_state(state, subdir, gitrepo)?;

    let gitrepo_content = gitrepo.format();
    apply_tree_into_subdir(
        repo,
        &state.workdir,
        subdir,
        merged_tree_id,
        &gitrepo_content,
        true,
        opts.force,
    )?;

    let message = match opts.message {
        Some(m) => m,
        None => build_default_commit_message(
            repo,
            opts.command,
            &[subdir.to_string()],
            subdir,
            &gitrepo.remote,
            &gitrepo.branch,
            Some(upstream_head),
            Some(merged_commit),
        )?,
    };

    git_commit_then_update_sync_ref(
        &state.workdir,
        repo,
        refs,
        CommitMessageSpec {
            message: Some(message),
            message_file: opts.message_file,
            edit: opts.edit,
        },
    )?;

    // Remove the linked worktree, indicating the operation is complete.
    remove_worktree(&state.workdir, &refs.branch, opts.command)?;

    repo.reference(
        refs.refs_commit.as_str(),
        merged_commit,
        gix::refs::transaction::PreviousValue::Any,
        "",
    )
    .into_subrepo_result()?;

    Ok(())
}

fn git_commit_then_update_sync_ref(
    repo_root: &Path,
    repo: &gix::Repository,
    refs: &SubrepoRefs,
    spec: CommitMessageSpec,
) -> Result<gix::ObjectId> {
    git_commit(repo_root, spec)?;
    let new_head = git_rev_parse_commit(repo_root, "HEAD")?;
    repo.reference(
        refs.refs_sync.as_str(),
        new_head,
        gix::refs::transaction::PreviousValue::Any,
        "",
    )
    .into_subrepo_result()?;
    Ok(new_head)
}

fn git_commit(repo_root: &Path, spec: CommitMessageSpec) -> Result<()> {
    if let Some(path) = spec.message_file {
        if spec.edit {
            git_cli::run_or_command_failed(repo_root, &["commit", "--edit", "--file", &path])?;
        } else {
            git_cli::run_or_command_failed(repo_root, &["commit", "--file", &path])?;
        }
        return Ok(());
    }

    let Some(msg) = spec.message else {
        return Err(Error::user("Missing commit message"));
    };

    if spec.edit {
        git_cli::run_or_command_failed(repo_root, &["commit", "--edit", "-m", &msg])?;
    } else {
        git_cli::run_or_command_failed(repo_root, &["commit", "-m", &msg])?;
    }

    Ok(())
}

fn apply_tree_into_subdir(
    repo: &gix::Repository,
    workdir: &Path,
    subdir: &str,
    tree_id: gix::ObjectId,
    gitrepo_content: &str,
    overwrite_existing: bool,
    allow_untracked_overwrite: bool,
) -> Result<()> {
    use std::collections::HashSet;
    use std::ffi::OsStr;
    use std::sync::atomic::{AtomicBool, Ordering};

    use gix::bstr::ByteSlice;

    fn is_quiet_mode() -> bool {
        matches!(
            std::env::var("GIT_SUBREPO_QUIET").as_deref(),
            Ok("1") | Ok("true") | Ok("yes")
        )
    }

    fn advise_untracked_enabled(repo: &gix::Repository) -> bool {
        repo.config_snapshot()
            .boolean("subrepo.adviseUntracked")
            .unwrap_or(true)
    }

    #[cfg(unix)]
    use std::os::unix::ffi::OsStrExt;

    struct EntrySpec {
        path: Vec<u8>,
        id: gix::ObjectId,
        flags: gix_index::entry::Flags,
        mode: gix_index::entry::Mode,
    }

    let prefix = format!("{subdir}/");
    let prefix_bstr = prefix.as_bytes().as_bstr();

    let mut index = repo
        .index_or_load_from_head_or_empty()
        .into_subrepo_result()?
        .into_owned();

    let mut old_paths: HashSet<Vec<u8>> = HashSet::new();
    for e in index.entries() {
        let p = e.path(&index);
        if p.starts_with(prefix_bstr) {
            old_paths.insert(p.to_vec());
        }
    }

    let mut old_parent_dirs: HashSet<Vec<u8>> = HashSet::new();
    for p in &old_paths {
        for (idx, b) in p.iter().enumerate() {
            if *b == b'/' {
                old_parent_dirs.insert(p[..idx].to_vec());
            }
        }
    }

    let tree_index = repo
        .index_from_tree(tree_id.as_ref())
        .into_subrepo_result()?;

    let mut specs: Vec<EntrySpec> = Vec::with_capacity(tree_index.entries().len() + 1);
    for e in tree_index.entries() {
        let p = e.path(&tree_index);
        let mut out = Vec::with_capacity(prefix.len() + p.len());
        out.extend_from_slice(prefix.as_bytes());
        out.extend_from_slice(p);
        specs.push(EntrySpec {
            path: out,
            id: e.id,
            flags: e.flags,
            mode: e.mode,
        });
    }

    let blob_id = repo
        .write_blob(gitrepo_content)
        .into_subrepo_result()?
        .detach();

    {
        let mut out = Vec::with_capacity(prefix.len() + ".gitrepo".len());
        out.extend_from_slice(prefix.as_bytes());
        out.extend_from_slice(b".gitrepo");
        specs.push(EntrySpec {
            path: out,
            id: blob_id,
            flags: gix_index::entry::Flags::empty(),
            mode: gix_index::entry::Mode::FILE,
        });
    }

    let mut new_paths: HashSet<Vec<u8>> = HashSet::new();
    let mut new_parent_dirs: HashSet<Vec<u8>> = HashSet::new();
    for s in &specs {
        new_paths.insert(s.path.clone());
        for (idx, b) in s.path.iter().enumerate() {
            if *b == b'/' {
                new_parent_dirs.insert(s.path[..idx].to_vec());
            }
        }
    }

    if !allow_untracked_overwrite {
        let mut excludes = repo
            .excludes(
                &index,
                None,
                gix_worktree::stack::state::ignore::Source::WorktreeThenIdMappingIfNotSkipped,
            )
            .into_subrepo_result()?;

        let mut untracked_files: Vec<Vec<u8>> = Vec::new();
        let mut conflicts: Vec<Vec<u8>> = Vec::new();
        let subdir_root = workdir.join(subdir);

        if subdir_root.is_dir() {
            let mut dirs = vec![subdir_root];
            while let Some(dir) = dirs.pop() {
                for entry in std::fs::read_dir(&dir)? {
                    let entry = entry?;
                    let ty = entry.file_type()?;
                    let path = entry.path();

                    let rel = path.strip_prefix(workdir).map_err(Error::internal)?;

                    #[cfg(unix)]
                    let rel_bytes: Vec<u8> = rel.as_os_str().as_bytes().to_vec();

                    #[cfg(not(unix))]
                    let rel_bytes: Vec<u8> = rel
                        .to_string_lossy()
                        .replace(std::path::MAIN_SEPARATOR, "/")
                        .into_bytes();

                    if ty.is_dir() {
                        let platform = excludes.at_path(rel, Some(gix_index::entry::Mode::DIR))?;
                        if platform.is_excluded() {
                            continue;
                        }

                        if new_paths.contains(&rel_bytes) && !old_parent_dirs.contains(&rel_bytes) {
                            conflicts.push(rel_bytes);
                            continue;
                        }

                        dirs.push(path);
                        continue;
                    }

                    if old_paths.contains(&rel_bytes) {
                        continue;
                    }

                    let platform = excludes.at_path(rel, None)?;
                    if platform.is_excluded() {
                        continue;
                    }

                    untracked_files.push(rel_bytes);
                }
            }
        }

        for u in &untracked_files {
            if new_paths.contains(u) || new_parent_dirs.contains(u) {
                conflicts.push(u.clone());
                continue;
            }

            for (idx, b) in u.iter().enumerate() {
                if *b != b'/' {
                    continue;
                }

                if new_paths.contains(&u[..idx].to_vec()) {
                    conflicts.push(u.clone());
                    break;
                }
            }
        }

        // Remove tracked paths that would block checkout due to directory <-> file transitions.
        let mut pre_delete: Vec<Vec<u8>> = Vec::new();
        for p in &old_paths {
            if new_parent_dirs.contains(p) {
                pre_delete.push(p.clone());
                continue;
            }

            for (idx, b) in p.iter().enumerate() {
                if *b != b'/' {
                    continue;
                }
                if new_paths.contains(&p[..idx].to_vec()) {
                    pre_delete.push(p.clone());
                    break;
                }
            }
        }

        // Delete deeper paths first so directory cleanup is safe.
        pre_delete.sort_by_key(|p| std::cmp::Reverse(p.len()));
        for path in &pre_delete {
            #[cfg(unix)]
            let os_path = OsStr::from_bytes(path);

            #[cfg(not(unix))]
            let os_path = OsStr::new(std::str::from_utf8(path).map_err(|_| {
                Error::user("Invalid path encoding while removing old tracked files")
            })?);

            let full = workdir.join(os_path);
            let meta = std::fs::symlink_metadata(&full);
            match meta {
                Ok(meta) if meta.is_dir() => {
                    let _ = std::fs::remove_dir_all(&full);
                }
                Ok(_) => {
                    let _ = std::fs::remove_file(&full);
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(err.into()),
            }

            // Remove empty parent directories up to `subdir/`.
            if let Ok(rel) = full.strip_prefix(workdir.join(subdir)) {
                let mut cur = rel.parent();
                while let Some(dir) = cur {
                    let abs = workdir.join(subdir).join(dir);
                    if abs.as_os_str().is_empty() {
                        break;
                    }
                    if abs
                        .read_dir()
                        .map(|mut it| it.next().is_none())
                        .unwrap_or(false)
                    {
                        let _ = std::fs::remove_dir(&abs);
                        cur = dir.parent();
                        continue;
                    }
                    break;
                }
            }
        }

        if !conflicts.is_empty() {
            conflicts.sort();
            conflicts.dedup();

            let mut msg = String::new();
            msg.push_str("Untracked working tree files would be overwritten by checkout:\n");
            for p in conflicts {
                msg.push_str("  ");
                msg.push_str(&String::from_utf8_lossy(&p));
                msg.push('\n');
            }

            return Err(Error::user(msg.trim_end().to_string()));
        }

        if !is_quiet_mode() && advise_untracked_enabled(repo) && !untracked_files.is_empty() {
            eprintln!(
                "git-subrepo: warning: '{}' contains non-ignored untracked files; consider updating .gitignore",
                subdir
            );
        }
    }

    // Checkout the new state using a temporary index containing only `subdir/` paths.
    let mut temp_state = gix_index::State::new(repo.object_hash());
    for e in &specs {
        temp_state.dangerously_push_entry(
            gix_index::entry::Stat::default(),
            e.id,
            e.flags,
            e.mode,
            e.path.as_bstr(),
        );
    }
    temp_state.sort_entries();

    let mut temp_index = gix_index::File::from_state(temp_state, repo.index_path());

    let mut opts = repo
        .checkout_options(gix_worktree::stack::state::attributes::Source::IdMapping)
        .into_subrepo_result()?;
    opts.destination_is_initially_empty = !overwrite_existing;
    opts.overwrite_existing = overwrite_existing;

    let should_interrupt = AtomicBool::new(false);
    let odb = repo.objects.clone().into_arc().into_subrepo_result()?;
    let outcome = gix_worktree_state::checkout(
        &mut temp_index,
        workdir,
        odb,
        &gix::progress::Discard,
        &gix::progress::Discard,
        &should_interrupt,
        opts,
    )
    .into_subrepo_result()?;

    if !outcome.errors.is_empty() {
        return Err(Error::user("Checkout failed."));
    }

    if !allow_untracked_overwrite && !outcome.collisions.is_empty() {
        let mut msg = String::new();
        msg.push_str("Checkout had collisions:\n");
        for c in outcome.collisions {
            msg.push_str("  ");
            msg.push_str(&c.path.to_string());
            msg.push('\n');
        }
        msg.push_str(
            "Note: the working tree may be partially updated. If this happens repeatedly, it may indicate a missing pre-check.\n",
        );
        return Err(Error::user(msg.trim_end().to_string()));
    }

    let mut to_delete: Vec<Vec<u8>> = old_paths.difference(&new_paths).cloned().collect();
    // Delete deeper paths first so directory cleanup is safe.
    to_delete.sort_by_key(|p| std::cmp::Reverse(p.len()));

    for path in &to_delete {
        #[cfg(unix)]
        let os_path = OsStr::from_bytes(path);

        #[cfg(not(unix))]
        let os_path =
            OsStr::new(std::str::from_utf8(path).map_err(|_| {
                Error::user("Invalid path encoding while removing old tracked files")
            })?);

        let full = workdir.join(os_path);
        let meta = std::fs::symlink_metadata(&full);
        match meta {
            Ok(meta) if meta.is_dir() => {
                let _ = std::fs::remove_dir_all(&full);
            }
            Ok(_) => {
                let _ = std::fs::remove_file(&full);
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }

        // Remove empty parent directories up to `subdir/`.
        if let Ok(rel) = full.strip_prefix(workdir.join(subdir)) {
            let mut cur = rel.parent();
            while let Some(dir) = cur {
                let abs = workdir.join(subdir).join(dir);
                if abs.as_os_str().is_empty() {
                    break;
                }
                if abs
                    .read_dir()
                    .map(|mut it| it.next().is_none())
                    .unwrap_or(false)
                {
                    let _ = std::fs::remove_dir(&abs);
                    cur = dir.parent();
                    continue;
                }
                break;
            }
        }
    }

    // Ensure the `.gitrepo` file content is present on disk (checkout writes it, but be explicit).
    std::fs::create_dir_all(workdir.join(subdir))?;
    std::fs::write(
        workdir.join(subdir).join(".gitrepo"),
        gitrepo_content.as_bytes(),
    )?;

    // Update main index: replace all entries under `subdir/`.
    index.remove_entries(|_, p, _| p.starts_with(prefix_bstr));

    for e in specs {
        #[cfg(unix)]
        let os_path = OsStr::from_bytes(&e.path);

        #[cfg(not(unix))]
        let os_path = OsStr::new(
            std::str::from_utf8(&e.path)
                .map_err(|_| Error::user("Invalid path encoding while updating index"))?,
        );

        let full = workdir.join(os_path);
        let meta = gix_index::fs::Metadata::from_path_no_follow(&full)?;
        let stat = gix_index::entry::Stat::from_fs(&meta).map_err(Error::internal)?;

        index.dangerously_push_entry(stat, e.id, e.flags, e.mode, e.path.as_bstr());
    }

    index.sort_entries();

    // We modify index entries directly. Invalidate the cache-tree extension so Git doesn't
    // accidentally write commits using stale tree data.
    let _ = index.remove_tree();

    index.write(Default::default()).into_subrepo_result()?;

    // Avoid reordering of operations by the compiler across file-system boundaries.
    std::sync::atomic::fence(Ordering::SeqCst);

    Ok(())
}

#[cfg(test)]
mod apply_tree_tests {
    use super::*;

    use std::path::Path;

    fn run_checked(cwd: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("git must run");
        if !out.status.success() {
            panic!(
                "git failed: git {}\nstdout:\n{}\nstderr:\n{}",
                args.join(" "),
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }

    #[test]
    fn tracked_only_force_delete_preserves_untracked() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let repo_dir = tmp.path().join("repo");
        std::fs::create_dir_all(&repo_dir).expect("mkdir");

        run_checked(&repo_dir, &["init", "-q"]);
        run_checked(&repo_dir, &["config", "user.name", "Test"]);
        run_checked(&repo_dir, &["config", "user.email", "test@test"]);

        std::fs::create_dir_all(repo_dir.join("sub")).expect("mkdir sub");
        std::fs::write(repo_dir.join("sub/tracked"), b"tracked\n").expect("write tracked");
        std::fs::write(repo_dir.join("sub/untracked"), b"untracked\n").expect("write untracked");

        run_checked(&repo_dir, &["add", "sub/tracked"]);
        run_checked(&repo_dir, &["commit", "-q", "-m", "init"]);

        let repo = gix::open(&repo_dir).expect("open repo");
        let empty_tree = repo.empty_tree().id;

        apply_tree_into_subdir(
            &repo,
            &repo_dir,
            "sub",
            empty_tree,
            GitRepoState {
                remote: "none".to_string(),
                branch: "master".to_string(),
                commit: "".to_string(),
                parent: "".to_string(),
                method: JoinMethod::Merge,
                cmdver: VERSION.to_string(),
            }
            .format()
            .as_str(),
            true,
            true,
        )
        .expect("apply");

        assert!(!repo_dir.join("sub/tracked").exists());
        assert!(repo_dir.join("sub/untracked").exists());
    }
}

fn build_default_commit_message(
    repo: &gix::Repository,
    command: &str,
    args: &[String],
    subdir: &str,
    remote: &str,
    branch: &str,
    upstream_commit: Option<gix::ObjectId>,
    merged_commit: Option<gix::ObjectId>,
) -> Result<String> {
    use gix::prelude::ObjectIdExt;

    let commit = upstream_commit
        .map(|id| id.attach(repo).shorten_or_id().to_string())
        .unwrap_or_else(|| "none".to_string());

    let merged = merged_commit
        .map(|id| id.attach(repo).shorten_or_id().to_string())
        .unwrap_or_else(|| "none".to_string());

    let is_merge = if command == "push" {
        false
    } else if let Some(id) = merged_commit {
        let commit = repo.find_commit(id).into_subrepo_result()?;
        commit.parent_ids().count() > 1
    } else {
        false
    };

    let merge_suffix = if is_merge { " (merge)" } else { "" };
    let args = args.join(" ");

    Ok(format!(
        "git subrepo {command}{merge_suffix} {args}\n\n\
subrepo:\n\
  subdir:   \"{subdir}\"\n\
  merged:   \"{merged}\"\n\
upstream:\n\
  origin:   \"{remote}\"\n\
  branch:   \"{branch}\"\n\
  commit:   \"{commit}\"\n\
git-subrepo:\n\
  version:  \"{VERSION}\"\n\
  origin:   \"unknown\"\n\
  commit:   \"unknown\"\n",
    ))
}
