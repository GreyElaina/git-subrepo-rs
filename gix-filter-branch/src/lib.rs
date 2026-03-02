use std::{collections::HashMap, error::Error as StdError, path::Path};

use gix::bstr::ByteSlice;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, FilterBranchError>;

#[derive(Debug, Error)]
pub enum FilterBranchError {
    #[error("{0}")]
    User(String),

    #[error(transparent)]
    Other(#[from] Box<dyn StdError + Send + Sync + 'static>),
}

impl FilterBranchError {
    pub fn user(message: impl Into<String>) -> Self {
        FilterBranchError::User(message.into())
    }
}

trait IntoOther<T> {
    fn into_other(self) -> Result<T>;
}

impl<T, E> IntoOther<T> for std::result::Result<T, E>
where
    E: StdError + Send + Sync + 'static,
{
    fn into_other(self) -> Result<T> {
        self.map_err(|err| FilterBranchError::Other(Box::new(err)))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub prune_empty: bool,
    pub remap_secondary_parents: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            prune_empty: true,
            remap_secondary_parents: true,
        }
    }
}

/// Equivalent to `git filter-branch --tree-filter "rm -f <path>" --prune-empty --first-parent`.
///
/// If `stop_at_exclusive` is set, it acts like limiting the range to `stop_at_exclusive..tip` along the first-parent chain.
pub fn tree_filter_remove_path_first_parent(
    repo: &gix::Repository,
    tip: gix::ObjectId,
    stop_at_exclusive: Option<gix::ObjectId>,
    path: &str,
    opts: Options,
) -> Result<gix::ObjectId> {
    let mut chain: Vec<gix::ObjectId> = Vec::new();
    let mut cur = Some(tip);
    while let Some(id) = cur {
        if stop_at_exclusive == Some(id) {
            break;
        }
        chain.push(id);
        let commit = repo.find_commit(id).into_other()?;
        cur = commit.parent_ids().next().map(|p| p.detach());
    }
    chain.reverse();

    let mut prev_new: Option<gix::ObjectId> = stop_at_exclusive;
    let mut prev_tree: Option<gix::ObjectId> = match stop_at_exclusive {
        Some(id) => {
            let commit = repo.find_commit(id).into_other()?;
            let tree_id = commit.tree_id().into_other()?.detach();
            Some(remove_path_from_tree(repo, tree_id, path)?)
        }
        None => Some(repo.empty_tree().id),
    };

    let mut map: HashMap<gix::ObjectId, gix::ObjectId> = HashMap::new();

    for id in chain {
        let commit = repo.find_commit(id).into_other()?;

        let message = commit.message_raw_sloppy().to_str_lossy().into_owned();

        let parent_ids: Vec<gix::ObjectId> = commit.parent_ids().map(|p| p.detach()).collect();
        let is_merge = parent_ids.len() > 1;

        let tree_id = commit.tree_id().into_other()?.detach();
        let new_tree_id = remove_path_from_tree(repo, tree_id, path)?;

        if opts.prune_empty && !is_merge {
            if let Some(prev_tree) = prev_tree {
                if new_tree_id == prev_tree {
                    if let Some(prev_new) = prev_new {
                        map.insert(id, prev_new);
                    }
                    continue;
                }
            }
        }

        let author_owned = commit.author().into_other()?.to_owned().into_other()?;
        let committer_owned = commit.committer().into_other()?.to_owned().into_other()?;

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
                if opts.remap_secondary_parents {
                    if let Some(remapped) = map.get(p) {
                        parents.push(*remapped);
                        continue;
                    }

                    if stop_at_exclusive == Some(*p) {
                        parents.push(*p);
                        continue;
                    }

                    // The parent commit was pruned or rewritten outside the current mapping.
                    // Skip it to avoid dangling references.
                    continue;
                }

                parents.push(*p);
            }
        }

        let new_commit = repo
            .new_commit_as(committer, author, message, new_tree_id, parents)
            .into_other()?;

        prev_new = Some(new_commit.id);
        prev_tree = Some(new_tree_id);
        map.insert(id, new_commit.id);
    }

    prev_new.ok_or_else(|| FilterBranchError::user("Filter produced no commits"))
}

/// Equivalent to `git filter-branch --subdirectory-filter <subdir> --prune-empty --first-parent`.
///
/// The rewritten history contains only the contents of `subdir`, with paths flattened to the root.
pub fn subdirectory_filter_first_parent(
    repo: &gix::Repository,
    tip: gix::ObjectId,
    stop_at_exclusive: Option<gix::ObjectId>,
    subdir: &str,
    opts: Options,
) -> Result<gix::ObjectId> {
    let empty_tree = repo.empty_tree().id;

    let mut chain: Vec<gix::ObjectId> = Vec::new();
    let mut cur = Some(tip);
    while let Some(id) = cur {
        if stop_at_exclusive == Some(id) {
            break;
        }
        chain.push(id);
        let commit = repo.find_commit(id).into_other()?;
        cur = commit.parent_ids().next().map(|p| p.detach());
    }
    chain.reverse();

    let mut prev_new: Option<gix::ObjectId> = stop_at_exclusive;
    let mut prev_tree: Option<gix::ObjectId> = match stop_at_exclusive {
        Some(id) => subtree_tree_id_at_commit(repo, id, subdir)?.or(Some(empty_tree)),
        None => Some(empty_tree),
    };

    let mut map: HashMap<gix::ObjectId, gix::ObjectId> = HashMap::new();

    for id in chain {
        let commit = repo.find_commit(id).into_other()?;

        let parent_ids: Vec<gix::ObjectId> = commit.parent_ids().map(|p| p.detach()).collect();
        let is_merge = parent_ids.len() > 1;

        let subtree_tree = subtree_tree_id_at_commit(repo, id, subdir)?.unwrap_or(empty_tree);

        if opts.prune_empty && !is_merge {
            if let Some(prev_tree) = prev_tree {
                if subtree_tree == prev_tree {
                    if let Some(prev_new) = prev_new {
                        map.insert(id, prev_new);
                    }
                    continue;
                }
            }
        }

        let author_owned = commit.author().into_other()?.to_owned().into_other()?;
        let committer_owned = commit.committer().into_other()?.to_owned().into_other()?;

        let mut author_time = gix::date::parse::TimeBuf::default();
        let author = author_owned.to_ref(&mut author_time);

        let mut committer_time = gix::date::parse::TimeBuf::default();
        let committer = committer_owned.to_ref(&mut committer_time);

        let message = commit.message_raw_sloppy().to_str_lossy().into_owned();

        let mut parents: Vec<gix::ObjectId> = Vec::new();
        if let Some(prev) = prev_new {
            parents.push(prev);
        }

        if is_merge {
            for p in parent_ids.iter().skip(1) {
                if opts.remap_secondary_parents {
                    if let Some(remapped) = map.get(p) {
                        parents.push(*remapped);
                        continue;
                    }

                    if stop_at_exclusive == Some(*p) {
                        parents.push(*p);
                        continue;
                    }

                    // The parent commit was pruned or rewritten outside the current mapping.
                    // Skip it to avoid dangling references.
                    continue;
                }

                parents.push(*p);
            }
        }

        let new_commit = repo
            .new_commit_as(committer, author, message, subtree_tree, parents)
            .into_other()?;

        prev_new = Some(new_commit.id);
        prev_tree = Some(subtree_tree);
        map.insert(id, new_commit.id);
    }

    prev_new.ok_or_else(|| FilterBranchError::user("Filter produced no commits"))
}

fn remove_path_from_tree(
    repo: &gix::Repository,
    tree_id: gix::ObjectId,
    path: &str,
) -> Result<gix::ObjectId> {
    let tree = repo.find_tree(tree_id).into_other()?;

    let entry = tree.lookup_entry_by_path(Path::new(path)).into_other()?;
    if entry.is_none() {
        return Ok(tree_id);
    }

    let mut editor = repo.edit_tree(tree_id).into_other()?;
    editor.remove(path).into_other()?;
    Ok(editor.write().into_other()?.detach())
}

fn subtree_tree_id_at_commit(
    repo: &gix::Repository,
    commit_id: gix::ObjectId,
    subdir: &str,
) -> Result<Option<gix::ObjectId>> {
    let commit = repo.find_commit(commit_id).into_other()?;
    let tree = commit.tree().into_other()?;
    let entry = tree.lookup_entry_by_path(Path::new(subdir)).into_other()?;

    let Some(entry) = entry else {
        return Ok(None);
    };

    if !entry.mode().is_tree() {
        return Ok(None);
    }

    Ok(Some(entry.object_id()))
}
