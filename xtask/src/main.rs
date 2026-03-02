use std::{
    env,
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
};

use anyhow::{anyhow, Context, Result};

fn main() {
    if let Err(err) = try_main() {
        eprintln!("xtask: {err}");
        std::process::exit(1);
    }
}

fn try_main() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();

    let cmd = args.first().map(String::as_str).unwrap_or("help");
    match cmd {
        "update-upstream-fixture" => update_upstream_fixture(&args[1..]),
        "help" | "-h" | "--help" => {
            print_help();
            Ok(())
        }
        other => Err(anyhow!("unknown xtask command: {other}")),
    }
}

fn print_help() {
    println!(
        "Usage:\n\
  cargo run -p xtask -- update-upstream-fixture [--release] [--allow-dirty]\n\
\n\
Commands:\n\
  update-upstream-fixture   Update git-subrepo/tests/upstream-fixture from its .gitrepo\n"
    );
}

fn update_upstream_fixture(args: &[String]) -> Result<()> {
    let mut release = false;
    let mut allow_dirty = false;

    for a in args {
        match a.as_str() {
            "--release" => release = true,
            "--allow-dirty" => allow_dirty = true,
            "-h" | "--help" => {
                print_help();
                return Ok(());
            }
            other => {
                return Err(anyhow!(
                    "unknown option for update-upstream-fixture: {other}"
                ))
            }
        }
    }

    let workspace_root = workspace_root()?;

    if !allow_dirty {
        ensure_git_clean(&workspace_root)?;
    }

    let fixture_path = workspace_root.join("git-subrepo/tests/upstream-fixture");
    if !fixture_path.is_dir() {
        return Err(anyhow!(
            "fixture path does not exist: {}",
            fixture_path.display()
        ));
    }

    let mut cargo_run = Command::new("cargo");
    cargo_run
        .arg("run")
        .arg("-p")
        .arg("git-subrepo")
        .args(release.then_some("--release"))
        .arg("--")
        .arg("pull")
        .arg("git-subrepo/tests/upstream-fixture");

    let status = run_cmd(&mut cargo_run, &workspace_root).context("run git subrepo pull")?;
    if !status.success() {
        return Err(anyhow!(
            "update failed (exit code: {}). If this is a conflict, resolve it in the linked worktree, then run: \n\
  cargo run -p git-subrepo -- commit git-subrepo/tests/upstream-fixture\n\
  cargo run -p git-subrepo -- clean git-subrepo/tests/upstream-fixture\n",
            exit_code(status)
        ));
    }

    println!(
        "\nNext steps:\n\
  git status\n\
  cargo nextest run -p git-subrepo --features upstream-tests\n"
    );

    Ok(())
}

fn workspace_root() -> Result<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest_dir
        .parent()
        .ok_or_else(|| anyhow!("xtask must be located at <workspace>/xtask"))?;
    Ok(root.to_path_buf())
}

fn ensure_git_clean(workspace_root: &Path) -> Result<()> {
    let out = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(workspace_root)
        .output()
        .context("git status")?;

    if !out.status.success() {
        return Err(anyhow!(
            "git status failed (exit code: {})",
            exit_code(out.status)
        ));
    }

    if !out.stdout.is_empty() {
        return Err(anyhow!(
            "working tree is dirty; commit/stash changes first, or re-run with --allow-dirty"
        ));
    }

    Ok(())
}

fn run_cmd(cmd: &mut Command, cwd: &Path) -> Result<ExitStatus> {
    cmd.current_dir(cwd);
    let status = cmd.status().with_context(|| format!("spawn: {cmd:?}"))?;
    Ok(status)
}

fn exit_code(status: ExitStatus) -> i32 {
    status.code().unwrap_or(1)
}
