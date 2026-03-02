use std::{collections::HashSet, iter::Peekable, process};

use anyhow::{anyhow, Result};

use git_subrepo_core::{
    BranchArgs, CleanArgs, CloneArgs, CommitArgs, ConfigArgs, FetchArgs, InitArgs, JoinMethod,
    PullArgs, PushArgs, StatusArgs,
};

#[derive(Debug, Default, Clone)]
struct Options {
    quiet: bool,

    all: bool,
    all_all: bool,

    fetch: bool,
    force: bool,
    squash: bool,
    update: bool,
    edit: bool,

    branch: Option<String>,
    remote: Option<String>,
    method: Option<JoinMethod>,

    message: Option<String>,
    file: Option<String>,
}

fn main() {
    if let Err(err) = try_main() {
        eprintln!("git-subrepo: {err}");
        process::exit(1);
    }
}

fn try_main() -> Result<()> {
    let argv: Vec<String> = std::env::args().skip(1).collect();

    if argv.len() == 1 && argv[0] == "--version" {
        println!("{}", git_subrepo_core::VERSION);
        return Ok(());
    }

    let ParsedArgs {
        opts,
        command,
        positionals,
    } = parse_argv(argv)?;

    if command == "version" {
        if !opts.quiet {
            println!("{}", git_subrepo_core::VERSION);
        }
        return Ok(());
    }

    validate_command(&command)?;
    validate_options(&command, &opts)?;

    if opts.message.is_some() && opts.file.is_some() {
        return Err(anyhow!(
            "fatal: options '-m' and '--file' cannot be used together"
        ));
    }

    if opts.update && opts.branch.is_none() && opts.remote.is_none() {
        return Err(anyhow!(
            "Can't use '--update' without '--branch' or '--remote'."
        ));
    }

    let include_nested = opts.all_all;

    let mut outputs: Vec<String> = Vec::new();

    match command.as_str() {
        "clone" => {
            let (remote, subdir, extra) =
                parse_two_positionals("clone", "remote", "subdir", &positionals)?;
            if !extra.is_empty() {
                return Err(anyhow!(
                    "Unknown argument(s) '{}' for 'clone' command.",
                    extra.join(" ")
                ));
            }

            let out = git_subrepo_core::clone(CloneArgs {
                remote,
                subdir,
                branch: opts.branch,
                force: opts.force,
                method: opts.method.unwrap_or(JoinMethod::Merge),
            })?;
            outputs.push(out);
        }
        "init" => {
            let (subdir, extra) = parse_one_positional("init", "subdir", &positionals)?;
            if !extra.is_empty() {
                return Err(anyhow!(
                    "Unknown argument(s) '{}' for 'init' command.",
                    extra.join(" ")
                ));
            }

            let out = git_subrepo_core::init(InitArgs {
                subdir,
                remote: opts.remote,
                branch: opts.branch,
                method: opts.method.unwrap_or(JoinMethod::Merge),
            })?;
            outputs.push(out);
        }
        "fetch" => {
            if opts.all {
                if !positionals.is_empty() {
                    return Err(anyhow!(
                        "Unknown argument(s) '{}' for 'fetch' command.",
                        positionals.join(" ")
                    ));
                }
                for subdir in git_subrepo_core::subrepos(include_nested)? {
                    let out = git_subrepo_core::fetch(FetchArgs {
                        subdir,
                        remote: opts.remote.clone(),
                        branch: opts.branch.clone(),
                    })?;
                    outputs.push(out);
                }
            } else {
                let (subdir, extra) = parse_one_positional("fetch", "subdir", &positionals)?;
                if !extra.is_empty() {
                    return Err(anyhow!(
                        "Unknown argument(s) '{}' for 'fetch' command.",
                        extra.join(" ")
                    ));
                }

                let out = git_subrepo_core::fetch(FetchArgs {
                    subdir,
                    remote: opts.remote,
                    branch: opts.branch,
                })?;
                outputs.push(out);
            }
        }
        "status" => {
            let subdir = positionals.first().cloned();
            if positionals.len() > 1 {
                let extra = positionals[1..].join(" ");
                return Err(anyhow!(
                    "Unknown argument(s) '{}' for 'status' command.",
                    extra
                ));
            }
            let out = git_subrepo_core::status(StatusArgs {
                subdir,
                all: opts.all,
                all_all: opts.all_all,
            })?;
            outputs.push(out);
        }
        "clean" => {
            if opts.all {
                if !positionals.is_empty() {
                    return Err(anyhow!(
                        "Unknown argument(s) '{}' for 'clean' command.",
                        positionals.join(" ")
                    ));
                }
                for subdir in git_subrepo_core::subrepos(include_nested)? {
                    let removed = git_subrepo_core::clean(CleanArgs {
                        subdir,
                        force: opts.force,
                    })?;
                    outputs.extend(removed);
                }
            } else {
                let (subdir, extra) = parse_one_positional("clean", "subdir", &positionals)?;
                if !extra.is_empty() {
                    return Err(anyhow!(
                        "Unknown argument(s) '{}' for 'clean' command.",
                        extra.join(" ")
                    ));
                }

                let removed = git_subrepo_core::clean(CleanArgs {
                    subdir,
                    force: opts.force,
                })?;
                outputs.extend(removed);
            }
        }
        "config" => {
            if positionals.len() < 2 {
                return Err(anyhow!("Command 'config' requires arg 'subdir'."));
            }
            let subdir = positionals[0].clone();
            let option = positionals[1].clone();
            let value = positionals.get(2).cloned();
            if positionals.len() > 3 {
                let extra = positionals[3..].join(" ");
                return Err(anyhow!(
                    "Unknown argument(s) '{}' for 'config' command.",
                    extra
                ));
            }

            let out = git_subrepo_core::config(ConfigArgs {
                subdir,
                option,
                value,
                force: opts.force,
            })?;
            outputs.push(out);
        }
        "branch" => {
            if opts.all {
                if !positionals.is_empty() {
                    return Err(anyhow!(
                        "Unknown argument(s) '{}' for 'branch' command.",
                        positionals.join(" ")
                    ));
                }
                for subdir in git_subrepo_core::subrepos(include_nested)? {
                    let out = git_subrepo_core::branch(BranchArgs {
                        subdir,
                        force: opts.force,
                        fetch: opts.fetch,
                    })?;
                    outputs.push(out);
                }
            } else {
                let (subdir, extra) = parse_one_positional("branch", "subdir", &positionals)?;
                if !extra.is_empty() {
                    return Err(anyhow!(
                        "Unknown argument(s) '{}' for 'branch' command.",
                        extra.join(" ")
                    ));
                }

                let out = git_subrepo_core::branch(BranchArgs {
                    subdir,
                    force: opts.force,
                    fetch: opts.fetch,
                })?;
                outputs.push(out);
            }
        }
        "pull" => {
            if opts.all {
                if !positionals.is_empty() {
                    return Err(anyhow!(
                        "Unknown argument(s) '{}' for 'pull' command.",
                        positionals.join(" ")
                    ));
                }
                for subdir in git_subrepo_core::subrepos(include_nested)? {
                    let out = git_subrepo_core::pull(PullArgs {
                        subdir,
                        force: opts.force,
                        remote: opts.remote.clone(),
                        branch: opts.branch.clone(),
                        update: opts.update,
                        message: opts.message.clone(),
                        message_file: opts.file.clone(),
                        edit: opts.edit,
                    })?;
                    outputs.push(out);
                }
            } else {
                let (subdir, extra) = parse_one_positional("pull", "subdir", &positionals)?;
                if !extra.is_empty() {
                    return Err(anyhow!(
                        "Unknown argument(s) '{}' for 'pull' command.",
                        extra.join(" ")
                    ));
                }

                let out = git_subrepo_core::pull(PullArgs {
                    subdir,
                    force: opts.force,
                    remote: opts.remote,
                    branch: opts.branch,
                    update: opts.update,
                    message: opts.message,
                    message_file: opts.file,
                    edit: opts.edit,
                })?;
                outputs.push(out);
            }
        }
        "push" => {
            if opts.all {
                if !positionals.is_empty() {
                    return Err(anyhow!(
                        "Unknown argument(s) '{}' for 'push' command.",
                        positionals.join(" ")
                    ));
                }
                for subdir in git_subrepo_core::subrepos(include_nested)? {
                    let out = git_subrepo_core::push(PushArgs {
                        subdir,
                        force: opts.force,
                        squash: opts.squash,
                        remote: opts.remote.clone(),
                        branch: opts.branch.clone(),
                        update: opts.update,
                        message: opts.message.clone(),
                        message_file: opts.file.clone(),
                    })?;
                    outputs.push(out);
                }
            } else {
                let (subdir, extra) = parse_one_positional("push", "subdir", &positionals)?;
                if !extra.is_empty() {
                    return Err(anyhow!(
                        "Unknown argument(s) '{}' for 'push' command.",
                        extra.join(" ")
                    ));
                }

                let out = git_subrepo_core::push(PushArgs {
                    subdir,
                    force: opts.force,
                    squash: opts.squash,
                    remote: opts.remote,
                    branch: opts.branch,
                    update: opts.update,
                    message: opts.message,
                    message_file: opts.file,
                })?;
                outputs.push(out);
            }
        }
        "commit" => {
            if positionals.is_empty() {
                return Err(anyhow!("Command 'commit' requires arg 'subdir'."));
            }
            let subdir = positionals[0].clone();
            let commit_ref = positionals.get(1).cloned();
            if positionals.len() > 2 {
                let extra = positionals[2..].join(" ");
                return Err(anyhow!(
                    "Unknown argument(s) '{}' for 'commit' command.",
                    extra
                ));
            }

            let out = git_subrepo_core::commit(CommitArgs {
                subdir,
                commit_ref,
                force: opts.force,
                message: opts.message,
                message_file: opts.file,
                edit: opts.edit,
            })?;
            outputs.push(out);
        }
        other => {
            return Err(anyhow!(
                "'{other}' is not a command. See 'git subrepo help'."
            ));
        }
    }

    if !opts.quiet {
        let out = outputs
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        if !out.is_empty() {
            println!("{out}");
        }
    }

    Ok(())
}

struct ParsedArgs {
    opts: Options,
    command: String,
    positionals: Vec<String>,
}

fn parse_argv(argv: Vec<String>) -> Result<ParsedArgs> {
    let mut opts = Options::default();
    let mut command: Option<String> = None;
    let mut positionals: Vec<String> = Vec::new();

    let mut stop_options = false;

    let mut it = argv.into_iter().peekable();
    while let Some(arg) = it.next() {
        if !stop_options && arg == "--" {
            stop_options = true;
            continue;
        }

        if !stop_options && arg.starts_with('-') && arg != "-" {
            parse_option(&arg, &mut it, &mut opts)?;
            continue;
        }

        if command.is_none() {
            command = Some(arg);
        } else {
            positionals.push(arg);
        }
    }

    let command = command.ok_or_else(|| anyhow!("missing command"))?;

    Ok(ParsedArgs {
        opts,
        command,
        positionals,
    })
}

fn parse_option(
    arg: &str,
    it: &mut Peekable<impl Iterator<Item = String>>,
    opts: &mut Options,
) -> Result<()> {
    if arg.starts_with("--") {
        parse_long_option(arg, it, opts)
    } else {
        parse_short_option(arg, it, opts)
    }
}

fn parse_long_option(
    arg: &str,
    it: &mut Peekable<impl Iterator<Item = String>>,
    opts: &mut Options,
) -> Result<()> {
    let (name, value) = match arg.split_once('=') {
        Some((k, v)) => (k, Some(v.to_string())),
        None => (arg, None),
    };

    match name {
        "--quiet" => opts.quiet = true,
        "--all" => opts.all = true,
        "--ALL" => {
            opts.all = true;
            opts.all_all = true;
        }
        "--fetch" => opts.fetch = true,
        "--force" => opts.force = true,
        "--squash" => opts.squash = true,
        "--update" => opts.update = true,
        "--edit" => opts.edit = true,

        "--branch" => {
            let v = match value {
                Some(v) => v,
                None => next_value(name, it)?,
            };
            opts.branch = Some(v);
        }
        "--remote" => {
            let v = match value {
                Some(v) => v,
                None => next_value(name, it)?,
            };
            opts.remote = Some(v);
        }
        "--method" => {
            let raw = match value {
                Some(v) => v,
                None => next_value(name, it)?,
            };
            opts.method = Some(raw.parse()?);
        }
        "--message" => {
            let v = match value {
                Some(v) => v,
                None => next_value(name, it)?,
            };
            opts.message = Some(v);
        }
        "--file" => {
            let v = match value {
                Some(v) => v,
                None => next_value(name, it)?,
            };
            opts.file = Some(v);
        }

        _ => {
            let key = name.trim_start_matches("--");
            return Err(anyhow!("error: unknown option `{key}'"));
        }
    }

    Ok(())
}

fn parse_short_option(
    arg: &str,
    it: &mut Peekable<impl Iterator<Item = String>>,
    opts: &mut Options,
) -> Result<()> {
    match arg {
        "-q" => opts.quiet = true,
        "-a" => opts.all = true,
        "-A" => {
            opts.all = true;
            opts.all_all = true;
        }
        "-F" => opts.fetch = true,
        "-f" => opts.force = true,
        "-s" => opts.squash = true,
        "-u" => opts.update = true,
        "-e" => opts.edit = true,

        "-b" => opts.branch = Some(next_value(arg, it)?),
        "-r" => opts.remote = Some(next_value(arg, it)?),
        "-M" => {
            let raw = next_value(arg, it)?;
            opts.method = Some(raw.parse()?);
        }
        "-m" => opts.message = Some(next_value(arg, it)?),

        _ => {
            if let Some(v) = arg.strip_prefix("-b") {
                if !v.is_empty() {
                    opts.branch = Some(v.to_string());
                    return Ok(());
                }
            }
            if let Some(v) = arg.strip_prefix("-r") {
                if !v.is_empty() {
                    opts.remote = Some(v.to_string());
                    return Ok(());
                }
            }
            if let Some(v) = arg.strip_prefix("-M") {
                if !v.is_empty() {
                    opts.method = Some(v.parse()?);
                    return Ok(());
                }
            }
            if let Some(v) = arg.strip_prefix("-m") {
                if !v.is_empty() {
                    opts.message = Some(v.to_string());
                    return Ok(());
                }
            }

            let key = arg.trim_start_matches('-');
            return Err(anyhow!("error: unknown option `{key}'"));
        }
    }

    Ok(())
}

fn next_value(name: &str, it: &mut Peekable<impl Iterator<Item = String>>) -> Result<String> {
    it.next().ok_or_else(|| anyhow!("Missing value for {name}"))
}

fn validate_command(command: &str) -> Result<()> {
    static COMMANDS: &[&str] = &[
        "clone", "init", "fetch", "pull", "push", "branch", "commit", "status", "clean", "config",
        "version",
    ];

    if COMMANDS.contains(&command) {
        return Ok(());
    }

    Err(anyhow!(
        "'{command}' is not a command. See 'git subrepo help'."
    ))
}

fn validate_options(command: &str, opts: &Options) -> Result<()> {
    let allowed = allowed_options(command);

    if opts.all && !allowed.contains("all") {
        return Err(anyhow!("Invalid option '--all' for '{command}'."));
    }
    if opts.all_all && !allowed.contains("ALL") {
        return Err(anyhow!("Invalid option '--ALL' for '{command}'."));
    }
    if opts.fetch && !allowed.contains("fetch") {
        return Err(anyhow!("Invalid option '--fetch' for '{command}'."));
    }
    if opts.force && !allowed.contains("force") {
        return Err(anyhow!("Invalid option '--force' for '{command}'."));
    }
    if opts.squash && !allowed.contains("squash") {
        return Err(anyhow!("Invalid option '--squash' for '{command}'."));
    }
    if opts.update && !allowed.contains("update") {
        return Err(anyhow!("Invalid option '--update' for '{command}'."));
    }
    if opts.edit && !allowed.contains("edit") {
        return Err(anyhow!("Invalid option '--edit' for '{command}'."));
    }

    if opts.branch.is_some() && !allowed.contains("branch") {
        return Err(anyhow!("Invalid option '--branch' for '{command}'."));
    }
    if opts.remote.is_some() && !allowed.contains("remote") {
        return Err(anyhow!("Invalid option '--remote' for '{command}'."));
    }
    if opts.method.is_some() && !allowed.contains("method") {
        return Err(anyhow!("Invalid option '--method' for '{command}'."));
    }
    if (opts.message.is_some() || opts.file.is_some()) && !allowed.contains("message") {
        return Err(anyhow!("Invalid option '--message' for '{command}'."));
    }

    Ok(())
}

fn allowed_options(command: &str) -> HashSet<&'static str> {
    match command {
        "clone" => HashSet::from(["branch", "edit", "force", "method", "message"]),
        "init" => HashSet::from(["branch", "method", "remote"]),
        "pull" => HashSet::from([
            "all", "branch", "edit", "fetch", "force", "message", "remote", "squash", "update",
        ]),
        "fetch" => HashSet::from(["all", "branch", "fetch", "remote"]),
        "push" => HashSet::from([
            "all", "branch", "force", "message", "remote", "squash", "update",
        ]),
        "branch" => HashSet::from(["all", "fetch", "force"]),
        "config" => HashSet::from(["force"]),
        "status" => HashSet::from(["all", "ALL"]),
        "clean" => HashSet::from(["all", "ALL", "force"]),
        "commit" => HashSet::from(["edit", "fetch", "force", "message"]),
        _ => HashSet::new(),
    }
}

fn parse_one_positional(
    command: &str,
    name: &str,
    positionals: &[String],
) -> Result<(String, Vec<String>)> {
    if positionals.is_empty() {
        return Err(anyhow!("Command '{command}' requires arg '{name}'."));
    }
    let first = positionals[0].clone();
    let extra = positionals[1..].to_vec();
    Ok((first, extra))
}

fn parse_two_positionals(
    command: &str,
    name1: &str,
    _name2: &str,
    positionals: &[String],
) -> Result<(String, Option<String>, Vec<String>)> {
    if positionals.is_empty() {
        return Err(anyhow!("Command '{command}' requires arg '{name1}'."));
    }

    let first = positionals[0].clone();
    let second = positionals.get(1).cloned();
    let extra = if positionals.len() > 2 {
        positionals[2..].to_vec()
    } else {
        Vec::new()
    };

    if positionals.len() == 1 {
        Ok((first, None, extra))
    } else {
        Ok((first, second, extra))
    }
}
