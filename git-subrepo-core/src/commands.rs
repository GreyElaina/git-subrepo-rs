use std::path::{Path, PathBuf};

use gix::bstr::ByteSlice;

use crate::{
    error::{Error, Result, SubrepoResultExt},
    git_cli,
    gitrepo::{GitRepoState, JoinMethod},
    refs::SubrepoRefs,
    remote,
    repo::{ensure_repo_is_ready, RepoState},
    subdir,
};

use gix_filter_branch as filter_branch;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct CloneArgs {
    pub remote: String,
    pub subdir: Option<String>,
    pub branch: Option<String>,
    pub force: bool,
    pub method: JoinMethod,
}

pub fn clone(args: CloneArgs) -> Result<String> {
    let repo = crate::repo::discover_repo()?;
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

    let head_commit = repo.head_commit().into_subrepo_result()?;
    let head_tree_id = head_commit.tree_id().into_subrepo_result()?.detach();

    let gitrepo_state = GitRepoState {
        remote: args.remote.clone(),
        branch: branch.clone(),
        commit: upstream_head_commit.to_string(),
        parent: state
            .head_commit
            .expect("head commit is required")
            .to_string(),
        method: args.method,
        cmdver: VERSION.to_string(),
    };
    let gitrepo_blob = repo
        .write_blob(gitrepo_state.format())
        .into_subrepo_result()?
        .detach();

    let mut sub_editor = repo.edit_tree(upstream_tree_id).into_subrepo_result()?;
    sub_editor
        .upsert(".gitrepo", gix::object::tree::EntryKind::Blob, gitrepo_blob)
        .into_subrepo_result()?;
    let new_subtree_id = sub_editor.write().into_subrepo_result()?;

    let mut editor = repo.edit_tree(head_tree_id).into_subrepo_result()?;
    editor.remove(&subdir).into_subrepo_result()?;
    editor
        .upsert(
            &subdir,
            gix::object::tree::EntryKind::Tree,
            new_subtree_id.detach(),
        )
        .into_subrepo_result()?;

    let new_tree_id = editor.write().into_subrepo_result()?;

    let commit_message = format!(
        "git subrepo clone {remote} {subdir}\n\n",
        remote = args.remote,
        subdir = subdir,
    );

    repo.commit(
        "HEAD",
        commit_message,
        new_tree_id.detach(),
        [state.head_commit.expect("head commit is required")],
    )
    .into_subrepo_result()?;

    repo.reference(
        refs.refs_commit.as_str(),
        upstream_head_commit,
        gix::refs::transaction::PreviousValue::Any,
        "",
    )
    .into_subrepo_result()?;

    git_cli::run_or_command_failed(&state.workdir, &["reset", "--hard"])?;

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
    let repo = crate::repo::discover_repo()?;
    let state = ensure_repo_is_ready(&repo, "init", true, true)?;

    let subdir = subdir::normalize_subdir(&args.subdir)?;
    assert_subdir_ready_for_init(&repo, &subdir)?;

    let remote = args.remote.unwrap_or_else(|| "none".to_string());
    let branch = args.branch.unwrap_or_else(|| "master".to_string());

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
}

pub fn fetch(args: FetchArgs) -> Result<String> {
    let repo = crate::repo::discover_repo()?;
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
}

pub fn status(args: StatusArgs) -> Result<String> {
    let repo = crate::repo::discover_repo()?;
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

    let mut out = String::new();
    out.push_str(&format!("{} subrepos:\n", subrepos.len()));
    for subdir in subrepos {
        out.push_str(&format!("Git subrepo '{subdir}':\n"));
    }

    Ok(out.trim_end().to_string())
}

pub fn subrepos(include_nested: bool) -> Result<Vec<String>> {
    let repo = crate::repo::discover_repo()?;
    ensure_repo_is_ready(&repo, "status", true, false)?;

    let mut subrepos = list_subrepos_from_head(&repo)?;
    if !include_nested {
        subrepos = filter_top_level_subrepos(subrepos);
    }
    subrepos.sort();
    Ok(subrepos)
}

pub struct CleanArgs {
    pub subdir: String,
    pub force: bool,
}

pub fn clean(args: CleanArgs) -> Result<Vec<String>> {
    let repo = crate::repo::discover_repo()?;
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
    let repo = crate::repo::discover_repo()?;
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
    let repo = crate::repo::discover_repo()?;
    let state = ensure_repo_is_ready(&repo, "branch", true, true)?;

    let subdir = subdir::normalize_subdir(&args.subdir)?;
    let subref = subdir::encode_subdir(&subdir)?;
    let refs = SubrepoRefs::new(&subref);

    if args.fetch {
        let _ = fetch(FetchArgs {
            subdir: subdir.clone(),
            remote: None,
            branch: None,
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
    let repo = crate::repo::discover_repo()?;
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

    // `--force` for pull behaves like `clone` upstream.
    if args.force {
        return clone(CloneArgs {
            remote: gitrepo.remote.clone(),
            subdir: Some(subdir.clone()),
            branch: Some(gitrepo.branch.clone()),
            force: true,
            method: gitrepo.method,
        });
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
    let repo = crate::repo::discover_repo()?;
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

    git_commit(
        &state.workdir,
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
    pub message: Option<String>,
    pub message_file: Option<String>,
    pub edit: bool,
}

pub fn commit(args: CommitArgs) -> Result<String> {
    let repo = crate::repo::discover_repo()?;
    let state = ensure_repo_is_ready(&repo, "commit", true, true)?;

    let subdir = subdir::normalize_subdir(&args.subdir)?;
    let subref = subdir::encode_subdir(&subdir)?;
    let refs = SubrepoRefs::new(&subref);

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

    let filtered_tip = filter_branch::subdirectory_filter_first_parent(
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

    let ls = git_cli::run_checked_stdout(&state.workdir, &["ls-files", "--", subdir])?;
    if !ls.is_empty() {
        let _ = git_cli::run_raw(&state.workdir, &["rm", "-r", "--", subdir])?;
    }

    git_cli::run_or_command_failed(
        &state.workdir,
        &[
            "read-tree",
            &format!("--prefix={subdir}"),
            "-u",
            subrepo_commit_ref,
        ],
    )?;

    gitrepo.commit = upstream_head.to_string();

    if upstream_head == merged_commit {
        gitrepo.parent = state
            .head_commit
            .expect("head commit is required")
            .to_string();
    }

    gitrepo.cmdver = VERSION.to_string();

    write_gitrepo_state(state, subdir, gitrepo)?;

    let gitrepo_rel = format!("{subdir}/.gitrepo");
    git_cli::run_or_command_failed(&state.workdir, &["add", "-f", "--", &gitrepo_rel])?;

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

    git_commit(
        &state.workdir,
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
