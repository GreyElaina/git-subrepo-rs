use anyhow::Result;
use clap::{Args, Parser, Subcommand, ValueEnum};

use git_subrepo_core::{
    BranchArgs, CleanArgs, CloneArgs, CommitArgs, ConfigArgs, FetchArgs, InitArgs, JoinMethod,
    PullArgs, PushArgs, StatusArgs,
};

#[derive(Debug, Parser)]
#[command(name = "git-subrepo")]
#[command(about = "A CLI for working with git-subrepo repositories")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Clone(CloneCmd),
    Init(InitCmd),
    Fetch(FetchCmd),
    Branch(BranchCmd),
    Pull(PullCmd),
    Push(PushCmd),
    Commit(CommitCmd),
    Status(StatusCmd),
    Clean(CleanCmd),
    Config(ConfigCmd),
    Version(VersionCmd),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum JoinMethodValue {
    Merge,
    Rebase,
}

impl From<JoinMethodValue> for JoinMethod {
    fn from(value: JoinMethodValue) -> Self {
        match value {
            JoinMethodValue::Merge => JoinMethod::Merge,
            JoinMethodValue::Rebase => JoinMethod::Rebase,
        }
    }
}

#[derive(Debug, Args)]
struct MessageArgs {
    #[arg(short = 'm', long = "message", conflicts_with = "message_file")]
    message: Option<String>,

    #[arg(long = "file", value_name = "PATH", conflicts_with = "message")]
    message_file: Option<String>,

    #[arg(short = 'e', long = "edit")]
    edit: bool,
}

#[derive(Debug, Args)]
struct CloneCmd {
    remote_url: String,

    subdir: Option<String>,

    #[arg(short = 'b', long = "branch")]
    branch: Option<String>,

    #[arg(short = 'f', long = "force")]
    force: bool,

    #[arg(long = "method", value_enum, default_value = "merge")]
    method: JoinMethodValue,

    #[command(flatten)]
    msg: MessageArgs,
}

#[derive(Debug, Args)]
struct InitCmd {
    subdir: String,

    #[arg(short = 'r', long = "remote")]
    remote: Option<String>,

    #[arg(short = 'b', long = "branch")]
    branch: Option<String>,

    #[arg(long = "method", value_enum, default_value = "merge")]
    method: JoinMethodValue,
}

#[derive(Debug, Args)]
struct FetchCmd {
    subdir: String,

    #[arg(short = 'r', long = "remote")]
    remote: Option<String>,

    #[arg(short = 'b', long = "branch")]
    branch: Option<String>,

    #[arg(short = 'f', long = "force")]
    force: bool,
}

#[derive(Debug, Args)]
struct BranchCmd {
    subdir: String,

    #[arg(short = 'f', long = "force")]
    force: bool,

    #[arg(short = 'F', long = "fetch")]
    fetch: bool,
}

#[derive(Debug, Args)]
struct PullCmd {
    subdir: String,

    #[arg(short = 'f', long = "force")]
    force: bool,

    #[arg(short = 'r', long = "remote")]
    remote: Option<String>,

    #[arg(short = 'b', long = "branch")]
    branch: Option<String>,

    #[arg(short = 'u', long = "update")]
    update: bool,

    #[command(flatten)]
    msg: MessageArgs,
}

#[derive(Debug, Args)]
struct PushCmd {
    subdir: String,

    #[arg(short = 'f', long = "force")]
    force: bool,

    #[arg(short = 's', long = "squash")]
    squash: bool,

    #[arg(short = 'r', long = "remote")]
    remote: Option<String>,

    #[arg(short = 'b', long = "branch")]
    branch: Option<String>,

    #[arg(short = 'u', long = "update")]
    update: bool,

    #[command(flatten)]
    msg: MessageArgs,
}

#[derive(Debug, Args)]
struct CommitCmd {
    subdir: String,

    subrepo_ref: Option<String>,

    #[arg(short = 'f', long = "force")]
    force: bool,

    #[arg(short = 'F', long = "fetch")]
    fetch: bool,

    #[command(flatten)]
    msg: MessageArgs,
}

#[derive(Debug, Args)]
struct StatusCmd {
    #[arg(value_name = "SUBDIR")]
    subdir: Option<String>,

    #[arg(long = "all")]
    all: bool,

    #[arg(long = "ALL")]
    all_all: bool,

    #[arg(short = 'F', long = "fetch")]
    fetch: bool,

    #[arg(short = 'q', long = "quiet")]
    quiet: bool,
}

#[derive(Debug, Args)]
struct CleanCmd {
    subdir: String,

    #[arg(short = 'f', long = "force")]
    force: bool,
}

#[derive(Debug, Args)]
struct ConfigCmd {
    subdir: String,

    option: String,

    value: Option<String>,

    #[arg(short = 'f', long = "force")]
    force: bool,
}

#[derive(Debug, Args)]
struct VersionCmd {
    #[arg(short = 'q', long = "quiet")]
    quiet: bool,
}

fn main() {
    if let Err(err) = try_main() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn try_main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Clone(cmd) => {
            let out = git_subrepo_core::clone(CloneArgs {
                remote: cmd.remote_url,
                subdir: cmd.subdir,
                branch: cmd.branch,
                force: cmd.force,
                method: cmd.method.into(),
                message: cmd.msg.message,
                message_file: cmd.msg.message_file,
                edit: cmd.msg.edit,
            })?;
            print_out(out);
        }
        Command::Init(cmd) => {
            let out = git_subrepo_core::init(InitArgs {
                subdir: cmd.subdir,
                remote: cmd.remote,
                branch: cmd.branch,
                method: cmd.method.into(),
            })?;
            print_out(out);
        }
        Command::Fetch(cmd) => {
            let out = git_subrepo_core::fetch(FetchArgs {
                subdir: cmd.subdir,
                remote: cmd.remote,
                branch: cmd.branch,
                force: cmd.force,
            })?;
            print_out(out);
        }
        Command::Branch(cmd) => {
            let out = git_subrepo_core::branch(BranchArgs {
                subdir: cmd.subdir,
                force: cmd.force,
                fetch: cmd.fetch,
            })?;
            print_out(out);
        }
        Command::Pull(cmd) => {
            let out = git_subrepo_core::pull(PullArgs {
                subdir: cmd.subdir,
                force: cmd.force,
                remote: cmd.remote,
                branch: cmd.branch,
                update: cmd.update,
                message: cmd.msg.message,
                message_file: cmd.msg.message_file,
                edit: cmd.msg.edit,
            })?;
            print_out(out);
        }
        Command::Push(cmd) => {
            let out = git_subrepo_core::push(PushArgs {
                subdir: cmd.subdir,
                force: cmd.force,
                squash: cmd.squash,
                remote: cmd.remote,
                branch: cmd.branch,
                update: cmd.update,
                message: cmd.msg.message,
                message_file: cmd.msg.message_file,
            })?;
            print_out(out);
        }
        Command::Commit(cmd) => {
            let out = git_subrepo_core::commit(CommitArgs {
                subdir: cmd.subdir,
                commit_ref: cmd.subrepo_ref,
                force: cmd.force,
                fetch: cmd.fetch,
                message: cmd.msg.message,
                message_file: cmd.msg.message_file,
                edit: cmd.msg.edit,
            })?;
            print_out(out);
        }
        Command::Status(cmd) => {
            if cmd.quiet {
                let include_nested = cmd.all_all;
                for s in git_subrepo_core::subrepos(include_nested)? {
                    println!("{s}");
                }
            } else {
                let out = git_subrepo_core::status(StatusArgs {
                    subdir: cmd.subdir,
                    all: cmd.all,
                    all_all: cmd.all_all,
                    fetch: cmd.fetch,
                })?;
                print_out(out);
            }
        }
        Command::Clean(cmd) => {
            let removed = git_subrepo_core::clean(CleanArgs {
                subdir: cmd.subdir,
                force: cmd.force,
            })?;
            if !removed.is_empty() {
                println!("{}", removed.join("\n"));
            }
        }
        Command::Config(cmd) => {
            let out = git_subrepo_core::config(ConfigArgs {
                subdir: cmd.subdir,
                option: cmd.option,
                value: cmd.value,
                force: cmd.force,
            })?;
            print_out(out);
        }
        Command::Version(cmd) => {
            if !cmd.quiet {
                println!("{}", git_subrepo_core::VERSION);
            }
        }
    }

    Ok(())
}

fn print_out(out: String) {
    if !out.is_empty() {
        println!("{out}");
    }
}
