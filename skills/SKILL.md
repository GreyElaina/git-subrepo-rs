---
name: git-subrepo
description: Usage guide for the `git subrepo` command (a Git submodule alternative) as implemented by this repository.
---

# git-subrepo

`git subrepo` lets you vendor an external Git repository into a subdirectory of your repository, and then:

- pull upstream changes into that subdirectory, and
- push local subdirectory changes back upstream.

A subrepo is identified by the presence of a `<subdir>/.gitrepo` file.

## When to use

Use `git subrepo` when you want a vendored copy of an external repository inside a subdirectory, but you want to avoid submodules.

Typical use-cases:

- vendor a library into `vendor/libname/`
- maintain local patches, occasionally syncing upstream
- contribute changes back upstream

## Key concepts

- **Subrepo directory**: the subdirectory that contains the vendored repository.
- **`.gitrepo`**: INI-like metadata file stored at `<subdir>/.gitrepo`.
- **Join method**: how the subrepo branch is joined with upstream during `pull`/`push` workflows (`merge` or `rebase`).
- **Linked worktree**: on conflicts, a linked worktree is created under `<git-common-dir>/tmp/subrepo/<subref>` to resolve conflicts.

## Common workflows

### Clone a subrepo into a subdirectory

```bash
git subrepo clone <remote-url> [<subdir>]
```

Example:

```bash
git subrepo clone https://github.com/org/project vendor/project
```

### Initialize an existing directory as a subrepo

```bash
git subrepo init <subdir>
```

If you know the upstream:

```bash
git subrepo init <subdir> --remote <remote-url> --branch <branch>
```

### Pull upstream changes

```bash
git subrepo pull <subdir>
```

Notes:

- If the subrepo is up to date, the command prints an up-to-date message.
- On conflicts, resolve them in the linked worktree and then run `git subrepo commit`.

### Push local changes upstream

```bash
git subrepo push <subdir>
```

### Conflict resolution workflow

When `pull` or `push` requires a merge/rebase that conflicts:

1. A linked worktree will exist at `<git-common-dir>/tmp/subrepo/<subref>`.
2. Resolve conflicts in that worktree:

```bash
cd <worktree>
git status
# resolve conflicts
git add -A
# if rebase:
git rebase --continue
# if merge:
git commit
```

3. Return to your original repo and finalize:

```bash
git subrepo commit <subdir>
git subrepo clean <subdir>
```

## Command reference

This implementation aims to match upstream semantics and output.

### `git subrepo clone <remote> [<subdir>]`

Clone the upstream repository into `<subdir>` and create `<subdir>/.gitrepo`.

Common options:

- `--branch <name>`
- `--force` (reclone)
- `--method <merge|rebase>`
- `--message <msg>` / `--file <path>` / `--edit`

### `git subrepo init <subdir>`

Turn an existing subdirectory into a subrepo.

Common options:

- `--remote <url>`
- `--branch <name>`
- `--method <merge|rebase>`

### `git subrepo fetch <subdir>`

Fetch the upstream content and update internal subrepo refs.

Common options:

- `--remote <url>`
- `--branch <name>`
- `--force`

### `git subrepo branch <subdir>`

Create a `subrepo/<subdir>` branch containing commits relevant to `<subdir>`.

Common options:

- `--fetch`
- `--force`

### `git subrepo commit <subdir> [<subrepo-ref>]`

Write the content of `<subrepo-ref>` (default: `subrepo/<subdir>`) into `<subdir>/` and create a single mainline commit.

Common options:

- `--fetch`
- `--force`
- `--message <msg>` / `--file <path>` / `--edit`

### `git subrepo pull <subdir>`

Fetch + branch + merge/rebase + commit.

Common options:

- `--remote <url>`
- `--branch <name>`
- `--update`
- `--force`
- `--message <msg>` / `--file <path>` / `--edit`

### `git subrepo push <subdir>`

Push local subrepo branch history upstream.

Common options:

- `--remote <url>`
- `--branch <name>`
- `--update`
- `--force`
- `--squash`
- `--message <msg>` / `--file <path>`

### `git subrepo status [<subdir>|--all|--ALL]`

Show subrepo presence/status.

Common options:

- `--fetch`
- `--quiet`

### `git subrepo clean <subdir>|--all|--ALL`

Remove artifacts created by `fetch` and `branch` (refs, branches, linked worktrees).

Common options:

- `--force`

### `git subrepo config <subdir> <option> [<value>]`

Read or update values in `<subdir>/.gitrepo`.

## Working tree safety (non-ignored untracked files)

When updating `<subdir>/` contents (e.g. `clone`, `pull`, `commit`), `git subrepo` will abort if a checkout would overwrite a **non-ignored untracked** path under `<subdir>/`.

Force operations allow overwriting.

### Configuration

Disable the untracked advisory warning:

```bash
git config subrepo.adviseUntracked false
```

`--quiet` suppresses advisory warnings.
