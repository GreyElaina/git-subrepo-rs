#![cfg(feature = "poc-tests")]

use std::{fs, path::Path, process::Command, sync::Mutex};

static CWD_LOCK: Mutex<()> = Mutex::new(());

use git_subrepo_core::{BranchArgs, GitRepoState, JoinMethod};

fn run_checked(cwd: &Path, args: &[&str]) {
    let out = Command::new("git")
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

fn run_stdout(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
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
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn write_text(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir parent");
    fs::write(path, content).expect("write file");
}

fn init_repo(repo: &Path) {
    run_checked(repo, &["init", "-q"]);
    run_checked(repo, &["config", "user.name", "Test"]);
    run_checked(repo, &["config", "user.email", "test@test"]);
}

fn first_parent_chain(repo: &Path, rev: &str) -> Vec<String> {
    let raw = run_stdout(repo, &["rev-list", "--first-parent", "--reverse", rev]);
    if raw.is_empty() {
        return Vec::new();
    }
    raw.lines().map(|l| l.trim().to_string()).collect()
}

fn tree_id(repo: &Path, oid: &str) -> String {
    run_stdout(repo, &["show", "-s", "--format=%T", oid])
}

fn has_gitrepo(repo: &Path, oid: &str) -> bool {
    let raw = run_stdout(repo, &["ls-tree", "--name-only", "-r", oid]);
    raw.lines().any(|l| l.trim() == ".gitrepo")
}

fn build_linear_fixture(repo: &Path) {
    // Create a subrepo dir with a .gitrepo file that has empty parent.
    let st = GitRepoState {
        remote: "none".to_string(),
        branch: "master".to_string(),
        commit: "".to_string(),
        parent: "".to_string(),
        method: JoinMethod::Merge,
        cmdver: git_subrepo_core::VERSION.to_string(),
    };

    write_text(&repo.join("bar/.gitrepo"), &st.format());
    write_text(&repo.join("bar/file"), "a\n");
    write_text(&repo.join("outside"), "o1\n");
    run_checked(repo, &["add", "-A"]);
    run_checked(repo, &["commit", "-q", "-m", "A"]);

    // Commit that changes only outside (should be pruned by subdirectory filter)
    write_text(&repo.join("outside"), "o2\n");
    run_checked(repo, &["add", "-A"]);
    run_checked(repo, &["commit", "-q", "-m", "B-outside"]);

    // Commit that changes only bar/file (must be kept)
    write_text(&repo.join("bar/file"), "b\n");
    run_checked(repo, &["add", "-A"]);
    run_checked(repo, &["commit", "-q", "-m", "C-bar"]);

    // Commit that changes only .gitrepo (should be pruned later)
    write_text(
        &repo.join("bar/.gitrepo"),
        &st.format().replace("none", "none2"),
    );
    run_checked(repo, &["add", "-A"]);
    run_checked(repo, &["commit", "-q", "-m", "D-only-gitrepo"]);

    // Another bar change (must be kept)
    write_text(&repo.join("bar/file"), "c\n");
    run_checked(repo, &["add", "-A"]);
    run_checked(repo, &["commit", "-q", "-m", "E-bar"]);
}

fn baseline_filter_branch(repo: &Path) {
    run_checked(repo, &["branch", "subrepo/bar", "HEAD"]);
    run_checked(
        repo,
        &[
            "filter-branch",
            "-f",
            "--subdirectory-filter",
            "bar",
            "subrepo/bar",
        ],
    );
    run_checked(
        repo,
        &[
            "filter-branch",
            "-f",
            "--prune-empty",
            "--tree-filter",
            "rm -f .gitrepo",
            "--",
            "subrepo/bar",
            "--first-parent",
        ],
    );
}

fn build_merge_fixture(repo: &Path) {
    let st = GitRepoState {
        remote: "none".to_string(),
        branch: "master".to_string(),
        commit: "".to_string(),
        parent: "".to_string(),
        method: JoinMethod::Merge,
        cmdver: git_subrepo_core::VERSION.to_string(),
    };

    write_text(&repo.join("bar/.gitrepo"), &st.format());
    write_text(&repo.join("bar/file"), "a\n");
    run_checked(repo, &["add", "-A"]);
    run_checked(repo, &["commit", "-q", "-m", "A"]);

    run_checked(repo, &["checkout", "-q", "-b", "side"]);
    write_text(&repo.join("bar/file"), "side\n");
    run_checked(repo, &["add", "-A"]);
    run_checked(repo, &["commit", "-q", "-m", "S1"]);

    run_checked(repo, &["checkout", "-q", "master"]);
    write_text(&repo.join("bar/file"), "main\n");
    run_checked(repo, &["add", "-A"]);
    run_checked(repo, &["commit", "-q", "-m", "M1"]);

    run_checked(
        repo,
        &["merge", "--no-ff", "-q", "side", "-m", "merge side"],
    );
}

#[test]
fn gix_rewrite_without_parent_matches_filter_branch_linear_history() {
    let tmp = tempfile::tempdir().expect("tempdir");

    let repo_gix = tmp.path().join("repo_gix");
    let repo_baseline = tmp.path().join("repo_baseline");

    fs::create_dir_all(&repo_gix).expect("mkdir");
    fs::create_dir_all(&repo_baseline).expect("mkdir");

    init_repo(&repo_gix);
    build_linear_fixture(&repo_gix);

    let out = Command::new("git")
        .args([
            "clone",
            "-q",
            repo_gix.to_string_lossy().as_ref(),
            repo_baseline.to_string_lossy().as_ref(),
        ])
        .current_dir(tmp.path())
        .output()
        .expect("git clone");
    if !out.status.success() {
        panic!(
            "git clone failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    // Run production code path (gix-rewrite feature): branch bar with empty parent.
    let _guard = CWD_LOCK.lock().expect("lock cwd");
    let cwd = std::env::current_dir().expect("cwd");
    std::env::set_current_dir(&repo_gix).expect("chdir");
    let _out = git_subrepo_core::branch(BranchArgs {
        subdir: "bar".to_string(),
        force: true,
        fetch: false,
    })
    .expect("branch");
    std::env::set_current_dir(cwd).expect("restore cwd");

    // Baseline: filter-branch pipeline.
    baseline_filter_branch(&repo_baseline);

    let gix_chain = first_parent_chain(&repo_gix, "subrepo/bar");
    let base_chain = first_parent_chain(&repo_baseline, "subrepo/bar");
    assert_eq!(gix_chain.len(), base_chain.len());

    for (g, b) in gix_chain.iter().zip(base_chain.iter()) {
        assert_eq!(tree_id(&repo_gix, g), tree_id(&repo_baseline, b));
        assert!(!has_gitrepo(&repo_gix, g));
        assert!(!has_gitrepo(&repo_baseline, b));
    }

    // Cleanup worktree produced by branch.
    let _ = git_subrepo_core::clean(git_subrepo_core::CleanArgs {
        subdir: "bar".to_string(),
        force: true,
    });
}

#[test]
fn gix_rewrite_without_parent_matches_filter_branch_with_merge_history() {
    let tmp = tempfile::tempdir().expect("tempdir");

    let repo_gix = tmp.path().join("repo_gix");
    let repo_baseline = tmp.path().join("repo_baseline");

    fs::create_dir_all(&repo_gix).expect("mkdir");
    fs::create_dir_all(&repo_baseline).expect("mkdir");

    init_repo(&repo_gix);
    build_merge_fixture(&repo_gix);

    let out = Command::new("git")
        .args([
            "clone",
            "-q",
            repo_gix.to_string_lossy().as_ref(),
            repo_baseline.to_string_lossy().as_ref(),
        ])
        .current_dir(tmp.path())
        .output()
        .expect("git clone");
    if !out.status.success() {
        panic!(
            "git clone failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let _guard = CWD_LOCK.lock().expect("lock cwd");
    let cwd = std::env::current_dir().expect("cwd");
    std::env::set_current_dir(&repo_gix).expect("chdir");
    let _out = git_subrepo_core::branch(BranchArgs {
        subdir: "bar".to_string(),
        force: true,
        fetch: false,
    })
    .expect("branch");
    std::env::set_current_dir(cwd).expect("restore cwd");

    baseline_filter_branch(&repo_baseline);

    let gix_count = run_stdout(&repo_gix, &["rev-list", "--count", "subrepo/bar"]);
    let base_count = run_stdout(&repo_baseline, &["rev-list", "--count", "subrepo/bar"]);
    assert_eq!(gix_count, base_count);

    let gix_merges = run_stdout(
        &repo_gix,
        &["rev-list", "--merges", "--count", "subrepo/bar"],
    );
    let base_merges = run_stdout(
        &repo_baseline,
        &["rev-list", "--merges", "--count", "subrepo/bar"],
    );
    assert_eq!(gix_merges, base_merges);

    let gix_tip = run_stdout(&repo_gix, &["rev-parse", "subrepo/bar"]);
    let base_tip = run_stdout(&repo_baseline, &["rev-parse", "subrepo/bar"]);
    assert_eq!(
        tree_id(&repo_gix, &gix_tip),
        tree_id(&repo_baseline, &base_tip)
    );

    assert!(!has_gitrepo(&repo_gix, &gix_tip));
    assert!(!has_gitrepo(&repo_baseline, &base_tip));

    let _ = git_subrepo_core::clean(git_subrepo_core::CleanArgs {
        subdir: "bar".to_string(),
        force: true,
    });
}
