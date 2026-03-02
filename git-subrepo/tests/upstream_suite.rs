#![cfg(all(unix, feature = "upstream-tests"))]

use std::{
    ffi::OsString,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{anyhow, Context, Result};

macro_rules! upstream_test {
    ($name:ident, $file:literal) => {
        #[test]
        fn $name() -> Result<()> {
            run_upstream_prove(&[concat!("test/", $file)])
        }
    };
}

upstream_test!(upstream_branch_all, "branch-all.t");
upstream_test!(
    upstream_branch_rev_list_one_path,
    "branch-rev-list-one-path.t"
);
upstream_test!(upstream_branch_rev_list, "branch-rev-list.t");
upstream_test!(upstream_branch, "branch.t");
upstream_test!(upstream_clean, "clean.t");
upstream_test!(upstream_clone_annotated_tag, "clone-annotated-tag.t");
upstream_test!(upstream_clone, "clone.t");
upstream_test!(upstream_compile, "compile.t");
upstream_test!(upstream_config, "config.t");
upstream_test!(upstream_encode, "encode.t");
upstream_test!(upstream_error, "error.t");
upstream_test!(upstream_fetch, "fetch.t");
upstream_test!(upstream_gitignore, "gitignore.t");
upstream_test!(upstream_init, "init.t");
upstream_test!(upstream_issue29, "issue29.t");
upstream_test!(upstream_issue95, "issue95.t");
upstream_test!(upstream_issue96, "issue96.t");
upstream_test!(upstream_pull_all, "pull-all.t");
upstream_test!(upstream_pull_merge, "pull-merge.t");
upstream_test!(upstream_pull_message, "pull-message.t");
upstream_test!(upstream_pull_new_branch, "pull-new-branch.t");
upstream_test!(upstream_pull_ours, "pull-ours.t");
upstream_test!(upstream_pull_theirs, "pull-theirs.t");
upstream_test!(upstream_pull_twice, "pull-twice.t");
upstream_test!(upstream_pull_worktree, "pull-worktree.t");
upstream_test!(upstream_pull, "pull.t");
upstream_test!(upstream_push_after_init, "push-after-init.t");
upstream_test!(
    upstream_push_after_push_no_changes,
    "push-after-push-no-changes.t"
);
upstream_test!(upstream_push_force, "push-force.t");
upstream_test!(upstream_push_new_branch, "push-new-branch.t");
upstream_test!(upstream_push_no_changes, "push-no-changes.t");
upstream_test!(upstream_push_squash, "push-squash.t");
upstream_test!(upstream_push, "push.t");
upstream_test!(upstream_rebase, "rebase.t");
upstream_test!(upstream_reclone, "reclone.t");
upstream_test!(upstream_status, "status.t");
upstream_test!(upstream_submodule, "submodule.t");
upstream_test!(upstream_zsh, "zsh.t");

fn run_upstream_prove(test_files: &[&str]) -> Result<()> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let upstream_src = manifest_dir.join("tests/upstream-fixture");

    if !upstream_src.is_dir() {
        return Err(anyhow!(
            "Upstream fixture '{}' not found.",
            upstream_src.display()
        ));
    }

    let temp_dir = tempfile::tempdir().context("create temp dir")?;
    let dst_root = temp_dir.path().join("git-subrepo-upstream");
    fs::create_dir_all(&dst_root).context("create temp upstream root")?;

    copy_tree(&upstream_src, &dst_root).context("copy upstream repo")?;

    init_git_repo(&dst_root).context("init temp upstream git repo")?;

    install_wrapper(&dst_root).context("install git-subrepo wrapper")?;
    patch_test_setup(&dst_root).context("patch upstream test setup")?;

    if test_files.is_empty() {
        return Err(anyhow!("Missing test file list."));
    }

    if test_files.len() != 1 {
        return Err(anyhow!(
            "Expected a single test file, got: {}",
            test_files.join(" ")
        ));
    }

    let mut cmd = Command::new("bash");
    cmd.arg(test_files[0]).current_dir(&dst_root);

    let wrapper_bin = dst_root.join("bin");
    let path_env = std::env::var_os("PATH").unwrap_or_default();
    let mut new_path = OsString::new();
    new_path.push(wrapper_bin.as_os_str());
    new_path.push(":");
    new_path.push(path_env);
    cmd.env("PATH", new_path);

    let out = cmd
        .output()
        .with_context(|| format!("run upstream test: {}", test_files.join(" ")))?;
    if out.status.success() {
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    Err(anyhow!(
        "Upstream test failed: {}\n\nstdout:\n{stdout}\n\nstderr:\n{stderr}",
        test_files.join(" ")
    ))
}

fn patch_test_setup(repo_root: &Path) -> Result<()> {
    let setup_path = repo_root.join("test/setup");
    let content = fs::read_to_string(&setup_path).context("read test/setup")?;

    let injection = "export PATH=\"$PWD/bin:$PATH\"";
    if content.contains(injection) {
        return Ok(());
    }

    let marker = "export PATH=$BASHLIB:$PATH";
    let Some(pos) = content.find(marker) else {
        return Err(anyhow!(
            "Failed to patch test/setup: marker '{marker}' not found"
        ));
    };

    let mut out = String::new();
    out.push_str(&content[..pos + marker.len()]);
    out.push('\n');
    out.push_str(injection);
    out.push('\n');
    out.push_str(&content[pos + marker.len()..]);

    fs::write(&setup_path, out).context("write patched test/setup")?;
    Ok(())
}

fn init_git_repo(repo_root: &Path) -> Result<()> {
    let out = Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(repo_root)
        .output()
        .context("git init")?;
    if out.status.success() {
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    Err(anyhow!(
        "git init failed.\n\nstdout:\n{stdout}\n\nstderr:\n{stderr}"
    ))
}

fn install_wrapper(repo_root: &Path) -> Result<()> {
    let bin_dir = repo_root.join("bin");
    fs::create_dir_all(&bin_dir).context("create bin dir")?;

    let wrapper_path = bin_dir.join("git-subrepo");

    let exe_path = env!("CARGO_BIN_EXE_git-subrepo-compat");
    let exe_quoted = bash_single_quote(exe_path);

    let content = format!(
        "#!/usr/bin/env bash\n\
set -euo pipefail\n\
exec {exe_quoted} \"$@\"\n"
    );

    fs::write(&wrapper_path, content).context("write wrapper")?;

    let mut perms = fs::metadata(&wrapper_path)
        .context("stat wrapper")?
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&wrapper_path, perms).context("chmod wrapper")?;

    Ok(())
}

fn copy_tree(src_root: &Path, dst_root: &Path) -> Result<()> {
    copy_tree_inner(src_root, src_root, dst_root)
}

fn copy_tree_inner(src_root: &Path, current: &Path, dst_root: &Path) -> Result<()> {
    for entry in
        fs::read_dir(current).with_context(|| format!("read dir: {}", current.display()))?
    {
        let entry = entry.context("read dir entry")?;
        let ty = entry.file_type().context("read file type")?;
        let src_path = entry.path();

        let rel = src_path.strip_prefix(src_root).context("strip prefix")?;

        if should_skip(rel) {
            continue;
        }

        let dst_path = dst_root.join(rel);

        if ty.is_dir() {
            fs::create_dir_all(&dst_path)
                .with_context(|| format!("create dir: {}", dst_path.display()))?;
            copy_tree_inner(src_root, &src_path, dst_root)?;
            continue;
        }

        if ty.is_file() {
            copy_file_with_permissions(&src_path, &dst_path)?;
            continue;
        }

        if ty.is_symlink() {
            let target = fs::read_link(&src_path)
                .with_context(|| format!("read symlink: {}", src_path.display()))?;
            std::os::unix::fs::symlink(&target, &dst_path)
                .with_context(|| format!("create symlink: {}", dst_path.display()))?;
            continue;
        }

        return Err(anyhow!("Unsupported file type: {}", src_path.display()));
    }

    Ok(())
}

fn copy_file_with_permissions(src: &Path, dst: &Path) -> Result<()> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create parent dir: {}", parent.display()))?;
    }

    fs::copy(src, dst).with_context(|| format!("copy file: {}", src.display()))?;

    let perms = fs::metadata(src)
        .with_context(|| format!("stat src: {}", src.display()))?
        .permissions();
    fs::set_permissions(dst, perms).with_context(|| format!("chmod dst: {}", dst.display()))?;

    Ok(())
}

fn should_skip(rel: &Path) -> bool {
    if rel.components().any(|c| c.as_os_str() == ".git") {
        return true;
    }

    // If the upstream fixture is managed as a subrepo inside this repository,
    // it will contain a root `.gitrepo` file which is not part of upstream.
    if rel == Path::new(".gitrepo") {
        return true;
    }

    if rel.starts_with("target") {
        return true;
    }

    if rel.starts_with("test/tmp") {
        return true;
    }

    false
}

fn bash_single_quote(s: &str) -> String {
    let mut out = String::new();
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}
