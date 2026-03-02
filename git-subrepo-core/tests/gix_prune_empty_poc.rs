#![cfg(feature = "poc-tests")]

use std::{fs, path::Path, process::Command};

use gix::bstr::ByteSlice;

use git_subrepo_core::{Error, Result, SubrepoResultExt};

fn run_checked(cwd: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git command must run");
    if !out.status.success() {
        panic!(
            "git failed: git {}\nstdout:\n{}\nstderr:\n{}",
            args.join(" "),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

fn run_stdout(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git command must run");
    if !out.status.success() {
        panic!(
            "git failed: git {}\nstdout:\n{}\nstderr:\n{}",
            args.join(" "),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn write_text(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().expect("parent")).expect("create parent dir");
    fs::write(path, content).expect("write file");
}

fn append_text(path: &Path, content: &str) {
    use std::io::Write;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent dir");
    }

    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("open file");
    f.write_all(content.as_bytes()).expect("append");
}

fn init_repo(repo: &Path) {
    run_checked(repo, &["init", "-q"]);
    run_checked(repo, &["config", "user.name", "Test"]);
    run_checked(repo, &["config", "user.email", "test@test"]);
}

fn make_fixture(repo: &Path) {
    // A: add file + .gitrepo
    write_text(&repo.join("f"), "a\n");
    write_text(&repo.join(".gitrepo"), "base\n");
    run_checked(repo, &["add", "-A"]);
    run_checked(repo, &["commit", "-q", "-m", "A"]);

    // B: change tracked file
    write_text(&repo.join("f"), "b\n");
    run_checked(repo, &["add", "-A"]);
    run_checked(repo, &["commit", "-q", "-m", "B"]);

    // C: change only .gitrepo (should be pruned)
    append_text(&repo.join(".gitrepo"), "c\n");
    run_checked(repo, &["add", "-A"]);
    run_checked(repo, &["commit", "-q", "-m", "C-only-gitrepo"]);

    // side: branch from B, change only .gitrepo (append-only to avoid conflict)
    run_checked(repo, &["checkout", "-q", "-b", "side", "HEAD~1"]);
    append_text(&repo.join(".gitrepo"), "side\n");
    run_checked(repo, &["add", "-A"]);
    run_checked(repo, &["commit", "-q", "-m", "S-only-gitrepo"]);

    // merge side into master without applying changes (to avoid merge conflicts).
    // The merge commit has two parents but doesn't change the tree (except .gitrepo),
    // and should not be pruned by prune-empty semantics.
    run_checked(repo, &["checkout", "-q", "master"]);
    run_checked(
        repo,
        &[
            "merge", "-q", "--no-ff", "-s", "ours", "side", "-m", "M-merge",
        ],
    );
}

fn make_fixture_for_parent_remap(repo: &Path) {
    // A
    write_text(&repo.join("f"), "a\n");
    write_text(&repo.join(".gitrepo"), "base\n");
    run_checked(repo, &["add", "-A"]);
    run_checked(repo, &["commit", "-q", "-m", "A"]);

    // B
    write_text(&repo.join("f"), "b\n");
    run_checked(repo, &["add", "-A"]);
    run_checked(repo, &["commit", "-q", "-m", "B"]);

    // C
    write_text(&repo.join("f"), "c\n");
    run_checked(repo, &["add", "-A"]);
    run_checked(repo, &["commit", "-q", "-m", "C"]);

    // Create a merge commit manually: parents are C (first) and B (second), tree equals C.
    let tree_c = run_stdout(repo, &["show", "-s", "--format=%T", "HEAD"]);
    let c = run_stdout(repo, &["rev-parse", "HEAD"]);
    let b = run_stdout(repo, &["rev-parse", "HEAD~1"]);

    let msg = "M-manual-merge\n";
    let mut child = Command::new("git")
        .args(["commit-tree", &tree_c, "-p", &c, "-p", &b])
        .current_dir(repo)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn commit-tree");

    use std::io::Write;
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(msg.as_bytes())
        .expect("write msg");

    let out = child.wait_with_output().expect("wait commit-tree");
    if !out.status.success() {
        panic!(
            "commit-tree failed stdout={} stderr={}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let m = String::from_utf8_lossy(&out.stdout).trim().to_string();
    run_checked(repo, &["update-ref", "refs/heads/master", &m]);
}

fn make_fixture_with_substantive_changes(repo: &Path) {
    write_text(&repo.join("f"), "a\n");
    write_text(&repo.join(".gitrepo"), "base\n");
    run_checked(repo, &["add", "-A"]);
    run_checked(repo, &["commit", "-q", "-m", "A"]);

    // B: changes both f and .gitrepo (must be kept)
    write_text(&repo.join("f"), "b\n");
    append_text(&repo.join(".gitrepo"), "b-meta\n");
    run_checked(repo, &["add", "-A"]);
    run_checked(repo, &["commit", "-q", "-m", "B-substantive+meta"]);

    // C: changes only .gitrepo (must be pruned)
    append_text(&repo.join(".gitrepo"), "c-meta\n");
    run_checked(repo, &["add", "-A"]);
    run_checked(repo, &["commit", "-q", "-m", "C-only-gitrepo"]);

    // D: changes only f (must be kept)
    write_text(&repo.join("f"), "d\n");
    run_checked(repo, &["add", "-A"]);
    run_checked(repo, &["commit", "-q", "-m", "D-substantive"]);

    // side: add a new file (no conflicts), merge normally
    run_checked(repo, &["checkout", "-q", "-b", "side", "HEAD~1"]);
    write_text(&repo.join("g"), "side\n");
    append_text(&repo.join(".gitrepo"), "side-meta\n");
    run_checked(repo, &["add", "-A"]);
    run_checked(repo, &["commit", "-q", "-m", "S-side-substantive+meta"]);

    run_checked(repo, &["checkout", "-q", "master"]);
    run_checked(repo, &["merge", "-q", "--no-ff", "side", "-m", "M-merge"]);
}

fn clone_local(src: &Path, dst: &Path) {
    let out = Command::new("git")
        .args([
            "clone",
            "-q",
            src.to_string_lossy().as_ref(),
            dst.to_string_lossy().as_ref(),
        ])
        .output()
        .expect("git clone");
    if !out.status.success() {
        panic!(
            "git clone failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

fn first_parent_chain(repo: &Path, rev: &str) -> Vec<String> {
    let raw = run_stdout(repo, &["rev-list", "--first-parent", "--reverse", rev]);
    if raw.is_empty() {
        return Vec::new();
    }
    raw.lines().map(|s| s.trim().to_string()).collect()
}

fn subject(repo: &Path, oid: &str) -> String {
    run_stdout(repo, &["show", "-s", "--format=%s", oid])
}

fn parents(repo: &Path, oid: &str) -> Vec<String> {
    let raw = run_stdout(repo, &["show", "-s", "--format=%P", oid]);
    if raw.is_empty() {
        return Vec::new();
    }
    raw.split_whitespace().map(|s| s.to_string()).collect()
}

fn tree_id(repo: &Path, oid: &str) -> String {
    run_stdout(repo, &["show", "-s", "--format=%T", oid])
}

fn blob_text(repo: &Path, oid: &str, path: &str) -> String {
    run_stdout(repo, &["show", &format!("{oid}:{path}")])
}

fn has_gitrepo(repo: &Path, oid: &str) -> bool {
    let raw = run_stdout(repo, &["ls-tree", "--name-only", "-r", oid]);
    raw.lines().any(|l| l.trim() == ".gitrepo")
}

fn gix_rewrite_remove_gitrepo_prune_empty_first_parent(
    repo: &gix::Repository,
    tip: gix::ObjectId,
    remap_secondary_parents: bool,
) -> Result<gix::ObjectId> {
    use std::collections::HashMap;

    let mut chain: Vec<gix::ObjectId> = Vec::new();
    let mut cur = Some(tip);
    while let Some(id) = cur {
        chain.push(id);
        let commit = repo.find_commit(id).into_subrepo_result()?;
        let mut parents = commit.parent_ids();
        cur = parents.next().map(|p| p.detach());
    }
    chain.reverse();

    let mut prev_new: Option<gix::ObjectId> = None;
    let mut prev_tree: Option<gix::ObjectId> = None;
    let mut map: HashMap<gix::ObjectId, gix::ObjectId> = HashMap::new();

    for id in chain {
        let commit = repo.find_commit(id).into_subrepo_result()?;
        let msg = commit.message_raw_sloppy().to_str_lossy().into_owned();

        let tree = commit.tree().into_subrepo_result()?;
        let mut editor = repo.edit_tree(tree.id).into_subrepo_result()?;
        let _ = editor.remove(".gitrepo");
        let new_tree = editor.write().into_subrepo_result()?.detach();

        let parent_ids: Vec<gix::ObjectId> = commit.parent_ids().map(|p| p.detach()).collect();
        let is_merge = parent_ids.len() > 1;

        if !is_merge {
            if let Some(prev_tree) = prev_tree {
                if new_tree == prev_tree {
                    continue;
                }
            }
        }

        let author_owned = commit
            .author()
            .into_subrepo_result()?
            .to_owned()
            .into_subrepo_result()?;
        let committer_owned = commit
            .committer()
            .into_subrepo_result()?
            .to_owned()
            .into_subrepo_result()?;

        let mut author_time = gix::date::parse::TimeBuf::default();
        let author = author_owned.to_ref(&mut author_time);

        let mut committer_time = gix::date::parse::TimeBuf::default();
        let committer = committer_owned.to_ref(&mut committer_time);

        let mut parents: Vec<gix::ObjectId> = Vec::new();
        if let Some(prev) = prev_new {
            parents.push(prev);
        }

        if is_merge {
            for p in parent_ids.iter().skip(1) {
                if remap_secondary_parents {
                    if let Some(remapped) = map.get(p) {
                        parents.push(*remapped);
                        continue;
                    }
                }
                parents.push(*p);
            }
        }

        let new_commit = repo
            .new_commit_as(committer, author, msg, new_tree, parents)
            .into_subrepo_result()?;

        map.insert(id, new_commit.id);
        prev_new = Some(new_commit.id);
        prev_tree = Some(new_tree);
    }

    prev_new.ok_or_else(|| Error::user("Rewrite produced no commits"))
}

fn filter_branch_remove_gitrepo_prune_empty_first_parent(repo_root: &Path) {
    run_checked(
        repo_root,
        &[
            "filter-branch",
            "-f",
            "--prune-empty",
            "--tree-filter",
            "rm -f .gitrepo",
            "--",
            "master",
            "--first-parent",
        ],
    );
}

#[test]
fn gix_prune_empty_matches_filter_branch_on_min_fixture() -> Result<()> {
    let tmp = tempfile::tempdir().expect("tempdir");
    let src = tmp.path().join("src");
    fs::create_dir_all(&src).expect("mkdir src");
    init_repo(&src);
    make_fixture(&src);

    let repo_filter = tmp.path().join("repo_filter");
    let repo_gix = tmp.path().join("repo_gix");

    clone_local(&src, &repo_filter);
    clone_local(&src, &repo_gix);

    filter_branch_remove_gitrepo_prune_empty_first_parent(&repo_filter);

    let filter_chain = first_parent_chain(&repo_filter, "master");
    assert!(!filter_chain.is_empty());

    let repo = gix::open(&repo_gix).into_subrepo_result()?;
    let tip = repo.head_commit().into_subrepo_result()?.id;

    let rewritten_tip = gix_rewrite_remove_gitrepo_prune_empty_first_parent(&repo, tip, false)?;
    repo.reference(
        "refs/heads/rewritten",
        rewritten_tip,
        gix::refs::transaction::PreviousValue::Any,
        "",
    )
    .into_subrepo_result()?;

    let gix_chain = first_parent_chain(&repo_gix, "rewritten");

    assert_eq!(filter_chain.len(), gix_chain.len(), "commit count differs");

    for (f, g) in filter_chain.iter().zip(gix_chain.iter()) {
        assert_eq!(parents(&repo_filter, f).len(), parents(&repo_gix, g).len());
        assert_eq!(tree_id(&repo_filter, f), tree_id(&repo_gix, g));
        assert!(!has_gitrepo(&repo_filter, f));
        assert!(!has_gitrepo(&repo_gix, g));
    }

    Ok(())
}

#[test]
fn gix_rewrite_remaps_secondary_parent_when_rewritten() -> Result<()> {
    let tmp = tempfile::tempdir().expect("tempdir");
    let src = tmp.path().join("src");
    fs::create_dir_all(&src).expect("mkdir src");
    init_repo(&src);
    make_fixture_for_parent_remap(&src);

    let repo_filter = tmp.path().join("repo_filter");
    let repo_gix = tmp.path().join("repo_gix");

    clone_local(&src, &repo_filter);
    clone_local(&src, &repo_gix);

    filter_branch_remove_gitrepo_prune_empty_first_parent(&repo_filter);

    let filter_chain = first_parent_chain(&repo_filter, "master");
    let filter_merge = filter_chain
        .iter()
        .find(|oid| subject(&repo_filter, oid) == "M-manual-merge")
        .expect("merge commit exists")
        .clone();
    let filter_b = filter_chain
        .iter()
        .find(|oid| subject(&repo_filter, oid) == "B")
        .expect("B exists")
        .clone();

    let filter_merge_parents = parents(&repo_filter, &filter_merge);
    assert_eq!(filter_merge_parents.len(), 2);
    assert_eq!(
        filter_merge_parents[1], filter_b,
        "filter-branch should remap secondary parent"
    );

    let repo = gix::open(&repo_gix).into_subrepo_result()?;
    let tip = repo.head_commit().into_subrepo_result()?.id;

    let rewritten_tip = gix_rewrite_remove_gitrepo_prune_empty_first_parent(&repo, tip, true)?;
    repo.reference(
        "refs/heads/rewritten",
        rewritten_tip,
        gix::refs::transaction::PreviousValue::Any,
        "",
    )
    .into_subrepo_result()?;

    let gix_chain = first_parent_chain(&repo_gix, "rewritten");
    let gix_merge = gix_chain
        .iter()
        .find(|oid| subject(&repo_gix, oid) == "M-manual-merge")
        .expect("merge commit exists")
        .clone();
    let gix_b = gix_chain
        .iter()
        .find(|oid| subject(&repo_gix, oid) == "B")
        .expect("B exists")
        .clone();

    let gix_merge_parents = parents(&repo_gix, &gix_merge);
    assert_eq!(gix_merge_parents.len(), 2);
    assert_eq!(
        gix_merge_parents[1], gix_b,
        "gix rewrite should remap secondary parent"
    );

    assert_eq!(filter_chain.len(), gix_chain.len());
    for (f, g) in filter_chain.iter().zip(gix_chain.iter()) {
        assert_eq!(parents(&repo_filter, f).len(), parents(&repo_gix, g).len());
        assert_eq!(tree_id(&repo_filter, f), tree_id(&repo_gix, g));
        assert!(!has_gitrepo(&repo_filter, f));
        assert!(!has_gitrepo(&repo_gix, g));
    }

    Ok(())
}

#[test]
fn gix_prune_empty_does_not_drop_substantive_changes() -> Result<()> {
    let tmp = tempfile::tempdir().expect("tempdir");
    let src = tmp.path().join("src");
    fs::create_dir_all(&src).expect("mkdir src");
    init_repo(&src);
    make_fixture_with_substantive_changes(&src);

    let repo_filter = tmp.path().join("repo_filter");
    let repo_gix = tmp.path().join("repo_gix");

    clone_local(&src, &repo_filter);
    clone_local(&src, &repo_gix);

    filter_branch_remove_gitrepo_prune_empty_first_parent(&repo_filter);

    let filter_chain = first_parent_chain(&repo_filter, "master");
    assert!(
        filter_chain
            .iter()
            .all(|oid| subject(&repo_filter, oid) != "C-only-gitrepo"),
        "filter chain should not contain pruned-only-gitrepo commit"
    );

    let repo = gix::open(&repo_gix).into_subrepo_result()?;
    let tip = repo.head_commit().into_subrepo_result()?.id;

    let rewritten_tip = gix_rewrite_remove_gitrepo_prune_empty_first_parent(&repo, tip, false)?;
    repo.reference(
        "refs/heads/rewritten",
        rewritten_tip,
        gix::refs::transaction::PreviousValue::Any,
        "",
    )
    .into_subrepo_result()?;

    let gix_chain = first_parent_chain(&repo_gix, "rewritten");
    assert!(
        gix_chain
            .iter()
            .all(|oid| subject(&repo_gix, oid) != "C-only-gitrepo"),
        "gix chain should not contain pruned-only-gitrepo commit"
    );

    assert_eq!(filter_chain.len(), gix_chain.len(), "commit count differs");

    for (f, g) in filter_chain.iter().zip(gix_chain.iter()) {
        assert_eq!(parents(&repo_filter, f).len(), parents(&repo_gix, g).len());
        assert_eq!(tree_id(&repo_filter, f), tree_id(&repo_gix, g));
        assert_eq!(
            blob_text(&repo_filter, f, "f"),
            blob_text(&repo_gix, g, "f")
        );
        assert!(!has_gitrepo(&repo_filter, f));
        assert!(!has_gitrepo(&repo_gix, g));
    }

    Ok(())
}
