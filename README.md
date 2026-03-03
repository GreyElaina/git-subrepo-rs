# git-subrepo-rs

`git-subrepo-rs` is a Rust reimplementation of the upstream Bash version of **git-subrepo** (a Git submodule alternative).

This repository provides a Rust implementation of `git subrepo` (a Git submodule alternative).

The core behavior is validated by running the upstream `.t` test suite against a compatibility harness in this repository.
The user-facing CLI in this repository uses a modern argument parser and does not aim for byte-for-byte CLI output compatibility with upstream.

## Name

`git-subrepo` - Git submodule alternative

## Synopsis

```bash
git subrepo --version

git subrepo clone <remote-url> [<subdir>]
git subrepo init <subdir>
git subrepo pull [<subdir>|--all|-A]
git subrepo push [<subdir>|--all|-A]

git subrepo fetch [<subdir>|--all|-A]
git subrepo branch [<subdir>|--all|-A]
git subrepo commit <subdir> [<subrepo-ref>]
git subrepo config <subdir> <option> [<value>]

git subrepo status [<subdir>|--all|-A]
git subrepo clean [<subdir>|--all|-A]

git subrepo patches [<subdir>] [--all|-A]
                 [--since <rev>|--from-ref <ref>|--since-sync]
                 [--style <style>]
                 [--reverse]

```

## Description

`git subrepo` lets you bring an external Git repository into a subdirectory of your repository, then:

- pull upstream changes into that subdirectory, and
- push local subdirectory changes back upstream.

A subrepo is identified by the presence of a `subdir/.gitrepo` file.

## Working tree safety

When updating a subrepo, this implementation protects **non-ignored untracked files** inside the subrepo directory:

- For non-force operations (`clone`, `pull`, `commit`), if a checkout would overwrite a non-ignored untracked file under `<subdir>/`, the command will abort (similar to Git’s checkout/merge safety checks).
- For force operations (`--force`), overwriting is allowed.

If `<subdir>/` contains non-ignored untracked files that do *not* conflict with the checkout, the command may emit an advisory warning.

### Configuration

- Disable the untracked advisory warning:

  ```bash
  git config subrepo.adviseUntracked false
  ```

- `--quiet` suppresses advisory warnings.

## Status

This project is primarily validated via the upstream `.t` test suite.

- A test-only compatibility harness (`git-subrepo-compat`) is built and used when running the upstream suite.
- The user-facing `git-subrepo` binary is intended to be pleasant to use (it supports `--help`) and may differ in error formatting or incidental output from upstream.

## Requirements

- Rust `1.82+`
- Git (upstream requires Git `>= 2.23`)
- `bash` (required by the upstream compatibility tests)
- (Optional) `docker` for the upstream `zsh.t` test (it will `skip_all` if Docker is unavailable)

## Installation

### Build from source

```bash
cd /path/to/git-subrepo-rs
cargo build --release
```

The binary will be located at:

```bash
/path/to/git-subrepo-rs/target/release/git-subrepo
```

### Make it available as `git subrepo`

Git discovers subcommands via executables named `git-<name>` in `PATH`.

```bash
ln -sf "/path/to/git-subrepo-rs/target/release/git-subrepo" "/usr/local/bin/git-subrepo"

git subrepo --version
```

## Commands

The underlying semantics are tested against the upstream suite, but the CLI output and help text may differ from upstream.

### `git subrepo clone <remote> [<subdir>]`

Add a repository as a subrepo into a subdirectory.

### `git subrepo init <subdir>`

Turn an existing subdirectory into a subrepo.

### `git subrepo fetch [<subdir>|--all|-A]`

Fetch the upstream content for a subrepo.

### `git subrepo branch [<subdir>|--all|-A]`

Create a subrepo branch containing local subrepo commits.

### `git subrepo pull [<subdir>|--all|-A]`

Pull upstream changes into the subrepo subdirectory.

### `git subrepo commit <subdir> [<subrepo-ref>]`

Commit the content of a subrepo branch into the mainline history.

### `git subrepo push [<subdir>|--all|-A]`

Push local subrepo changes upstream.

### `git subrepo status [<subdir>|--all|-A]`

Show status for one subrepo or multiple subrepos.

### `git subrepo clean [<subdir>|--all|-A]`

Remove artifacts created by `fetch` and `branch` (and commands that call them).

### `git subrepo config <subdir> <option> [<value>]`

Read or update configuration values in `subdir/.gitrepo`.

### `git subrepo patches [<subdir>] [--all|-A]`

List local mainline commits affecting a subrepo subdirectory since the last sync.

This command prints the base commit (not counted as a patch), followed by the local patch commits.

If `SUBDIR` is omitted, this command lists patches for all top-level subrepos (equivalent to `--all`).

- Default base: `refs/subrepo/<subref>/sync` (if present). If missing, `patches` will try to initialize it from `SUBDIR/.gitrepo` (`subrepo.parent`).
  As a fallback, it uses a commit-message anchor.
- Use `--since` or `--from-ref` for explicit control.

## Testing

### Fast tests

```bash
cargo nextest run
```

### Upstream compatibility suite (authoritative)

The upstream test suite is vendored under:

- `git-subrepo/tests/upstream-fixture`

Run it with:

```bash
cargo nextest run -p git-subrepo --features upstream-tests
```

Note: the upstream suite runs against a test-only compatibility harness binary (`git-subrepo-compat`).

### Conformance experiments

This repository contains additional tests used to validate the behavior of history filtering primitives.

```bash
cargo nextest run -p git-subrepo-core --features poc-tests
```

## License

MIT OR Apache-2.0
