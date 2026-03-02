---
name: git-subrepo
description: >
  Provides step-by-step usage guidance for the git-subrepo CLI tool.
  Use when the user wants to clone, pull, push, fetch, branch, commit,
  init, clean, or configure a subrepo; or when diagnosing subrepo errors
  or conflicts in this repository.
---

# git-subrepo

`git subrepo` lets you vendor an external Git repository into a subdirectory of your repository, and then:

- pull upstream changes into that subdirectory, and
- push local subdirectory changes back upstream.

A subrepo is identified by the presence of a `<subdir>/.gitrepo` file.

## Contents

- [When to use](#when-to-use)
- [Key concepts](#key-concepts)
- [Common workflows](#common-workflows)
- [History model](#history-model)
- [Command reference](#command-reference)
- [Known pitfalls](#known-pitfalls)
- [Working tree safety](#working-tree-safety-non-ignored-untracked-files)

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

### `.gitrepo` example

```ini
; DO NOT EDIT (unless you know what you are doing)
;
; This subdirectory is a git "subrepo", and this file is maintained by the
; git-subrepo command. See https://github.com/ingydotnet/git-subrepo#readme
;
[subrepo]
    remote = https://github.com/org/lib.git
    branch = main
    commit = a1b2c3d4
    parent = e5f6g7h8
    method = merge
    cmdver = 0.4.9
```

Field meanings:

- `remote`: upstream URL
- `branch`: upstream branch
- `commit`: last synced upstream commit SHA
- `parent`: mainline commit SHA that performed the last sync
- `method`: `merge` or `rebase`

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

After `init`, you typically push to an empty upstream repository:

```bash
git subrepo push <subdir> --remote <remote-url>
```

### Pull upstream changes

```bash
git subrepo pull <subdir>
```

If the operation conflicts, resolve in the linked worktree and then finalize:

```bash
git subrepo commit <subdir>
git subrepo clean <subdir>
```

### Push local changes upstream

```bash
git subrepo push <subdir>
```

### Conflict resolution workflow (linked worktree)

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

## History model

### Mainline commits are squashed

`pull` and `commit` write the subrepo contents into `<subdir>/` as a single commit in your current branch.

The full upstream history is preserved in internal refs and can be inspected with:

```bash
git log refs/subrepo/<subref>/fetch
```

### Push reconstructs a branch

`push` internally runs `git subrepo branch` to scan mainline history and
reconstruct individual commits, then pushes that branch upstream.

This means **mainline rebases that touch `<subdir>/` will invalidate the
`parent` field** — see [Known pitfalls](#known-pitfalls).

## Command reference

### `git subrepo clone <remote> [<subdir>]`

Common options:

- `--branch <name>`
- `--force` (reclone)
- `--method <merge|rebase>`
- `--message <msg>` / `--file <path>` / `--edit`

### `git subrepo init <subdir>`

Common options:

- `--remote <url>`
- `--branch <name>`
- `--method <merge|rebase>`

### `git subrepo fetch <subdir>`

Common options:

- `--remote <url>`
- `--branch <name>`
- `--force`

### `git subrepo branch <subdir>`

Common options:

- `--fetch`
- `--force`

### `git subrepo pull <subdir>`

Common options:

- `--remote <url>`
- `--branch <name>`
- `--update`
- `--force`
- `--message <msg>` / `--file <path>` / `--edit`

### `git subrepo commit <subdir> [<subrepo-ref>]`

Common options:

- `--fetch`
- `--force`
- `--message <msg>` / `--file <path>` / `--edit`

### `git subrepo push <subdir>`

Common options:

- `--remote <url>`
- `--branch <name>`
- `--update`
- `--force`
- `--squash`
- `--message <msg>` / `--file <path>`

### `git subrepo status [<subdir>|--all|--ALL]`

Common options:

- `--fetch`
- `--quiet`

### `git subrepo clean <subdir>|--all|--ALL`

Common options:

- `--force`

### `git subrepo config <subdir> <option> [<value>]`

Common options:

- `--force` (required for changing autogenerated fields)

## Known pitfalls

- **Mainline rebase breaks push**: if mainline commits touching `<subdir>/` are rebased after a `pull`, the `.gitrepo` `parent` field can become stale and `push` may fail. Fix: update `parent` in `.gitrepo` or run:

  ```bash
  git subrepo config <subdir> parent <sha> --force
  ```

- **Finalize after manual conflict resolution**: after resolving conflicts in the linked worktree, run both:

  ```bash
  git subrepo commit <subdir>
  git subrepo clean <subdir>
  ```

  Skipping `clean` can leave stale refs/worktrees.

- **`init` without `--remote`**: `init` without `--remote` creates a `.gitrepo` with no upstream configured; `pull`/`push` will fail until you configure it:

  ```bash
  git subrepo config <subdir> remote <url> --force
  git subrepo config <subdir> branch <branch> --force
  ```

- **Nested subrepos are not supported.** `status --ALL` and `clean --ALL`
  can discover all subrepos recursively, but workflows involving
  subrepos-within-subrepos are undefined.

## Working tree safety (non-ignored untracked files)

When updating `<subdir>/` contents (e.g. `clone`, `pull`, `commit`), `git subrepo` aborts if a checkout would overwrite a **non-ignored untracked** path under `<subdir>/`.

If non-ignored untracked files exist but would *not* be overwritten, `git subrepo` emits an advisory warning (non-fatal) instead of aborting.

Force operations allow overwriting.

### Configuration

Disable the untracked advisory warning:

```bash
git config subrepo.adviseUntracked false
```

`--quiet` suppresses advisory warnings.
