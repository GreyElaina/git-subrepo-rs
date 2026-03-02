---
name: git-subrepo-rs
description: Rust reimplementation of upstream Bash git-subrepo with output/behavior compatibility enforced by the upstream .t suite (run via cargo nextest).
---

# git-subrepo-rs

This skill documents the project-specific contract, layout, workflows, and guardrails for contributing to the Rust reimplementation of upstream Bash `git-subrepo`.

## Contract (authoritative)

- Behavioral and output compatibility with upstream Bash `git-subrepo`.
- The vendored upstream `.t` test suite is the authoritative specification.
- Prefer gitoxide (`gix`) for plumbing, but use the `git` CLI where porcelain UX/state parity is required.

## Repository layout

Workspace root contains these crates:

- `git-subrepo/`
  - `src/main.rs`: CLI argument parsing and dispatch.
  - `tests/upstream_suite.rs`: runs upstream tests from a temporary copy.
- `git-subrepo-core/`
  - `src/commands.rs`: command implementations (clone/init/fetch/branch/pull/push/commit/status/clean/config).
  - `src/gitrepo.rs`: `.gitrepo` parsing/formatting.
  - `src/remote.rs`: upstream fetch via `gix` remotes.
- `gix-filter-branch/`
  - History rewriting primitives used to match `git filter-branch` semantics.
- `xtask/`
  - Developer workflows (e.g. updating the upstream fixture).

Upstream fixture:

- `git-subrepo/tests/upstream-fixture`
  - This repository may be managed as a subrepo and can contain a root `.gitrepo` which must be ignored by the upstream test runner when copying to a temporary repo.

## How upstream tests are executed

- Test runner: `git-subrepo/tests/upstream_suite.rs`
  - Copies `tests/upstream-fixture` into a temp dir.
  - Initializes a new Git repo in the temp dir.
  - Injects a `bin/git-subrepo` wrapper into `PATH` so the Rust binary is used.
  - Skips copying the fixture root `.gitrepo` file.

## Commands (developer)

```bash
# Fast local tests
cargo nextest run

# Upstream compatibility suite (authoritative)
cargo nextest run -p git-subrepo --features upstream-tests

# Conformance/POC tests
cargo nextest run -p git-subrepo-core --features poc-tests

# Update the vendored upstream fixture
cargo run -p xtask -- update-upstream-fixture
```

## Implementation strategy

### Prefer `gix` for plumbing

Typical `gix`-friendly areas:

- reference and object manipulation
- tree/index/worktree checkout
- remote fetch (including HTTP transport)
- history traversal and rewriting (within the project-defined compatibility envelope)

### Use `git` CLI for porcelain parity

Prefer the `git` CLI where UX/state parity matters and is difficult to reproduce reliably:

- linked worktrees (`git worktree add/prune`)
- merge/rebase and conflict state
- push to network remotes
- commit message editing (`-e`) and file-based messages (`--file`), including hook execution

## Conflict workflow (upstream-compatible)

- A linked worktree is created under `<git-common-dir>/tmp/subrepo/<subref>`.
- If merge/rebase conflicts occur, users resolve them manually in the linked worktree.
- Completion sequence:

```bash
git subrepo commit <subdir>
git subrepo clean <subdir>
```

- Cleanup includes a dirty-guard for tracked changes.

## Working tree safety (non-ignored untracked files)

When writing `<subdir>/` contents (e.g. `clone`, `pull`, `commit`), the implementation protects non-ignored untracked files:

- Non-force operations abort if a checkout would overwrite a non-ignored untracked path under `<subdir>/`.
- Force operations allow overwriting.

Configuration:

```bash
# Disable untracked advisory warnings
git config subrepo.adviseUntracked false
```

`--quiet` suppresses advisory warnings.

## `.gitrepo` format and parsing

- `.gitrepo` is required to identify a subrepo.
- Parsing should be tolerant where upstream is tolerant:
  - `subrepo.parent` may be missing (treated as empty)
  - `subrepo.method` may be missing (defaults to `merge`)

## gix-filter-branch primitives

These helpers are used to emulate filter-branch-like semantics:

- `subdirectory_filter(...)`: rewrite the reachable commit graph while keeping merges.
- `tree_filter_remove_path_first_parent(...)`: remove a path on the first-parent chain (used for `.gitrepo` removal in certain flows).

Keep these properties:

- preserve author/committer metadata per commit
- pruning behavior matches the intended Git semantics (`--prune-empty` expectations)

## Change checklist

1. Prefer test-first updates: upstream suite is the spec.
2. Keep output strings stable (punctuation and quoting matter).
3. Run:

```bash
cargo fmt
cargo nextest run -p git-subrepo --features upstream-tests
```

4. Avoid documenting internal development decisions in `README.md` (project-facing only).
