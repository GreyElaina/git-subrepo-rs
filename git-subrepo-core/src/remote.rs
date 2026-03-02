use std::sync::atomic::AtomicBool;

use gix::bstr::ByteSlice;

use crate::{
    error::{Error, Result, SubrepoResultExt},
    refs::SubrepoRefs,
};

pub fn fetch_upstream_commit(
    repo: &gix::Repository,
    remote_url: &str,
    branch: &str,
    refs: &SubrepoRefs,
) -> Result<gix::ObjectId> {
    let candidates = if branch.starts_with("refs/") || branch == "HEAD" {
        vec![branch.to_string()]
    } else {
        vec![
            format!("refs/heads/{branch}"),
            format!("refs/tags/{branch}"),
            branch.to_string(),
        ]
    };

    let mut last_err = None;
    for src in candidates {
        match fetch_ref(repo, remote_url, &src, &refs.refs_fetch) {
            Ok(id) => return Ok(id),
            Err(err) => last_err = Some(err),
        }
    }

    Err(last_err.unwrap_or_else(|| Error::user("Fetch failed")))
}

fn fetch_ref(
    repo: &gix::Repository,
    remote_url: &str,
    src_ref: &str,
    dst_ref: &str,
) -> Result<gix::ObjectId> {
    let spec = format!("+{src_ref}:{dst_ref}");
    let remote = repo
        .remote_at(remote_url)
        .into_subrepo_result()?
        .with_fetch_tags(gix::remote::fetch::Tags::None)
        .with_refspecs([spec.as_bytes().as_bstr()], gix::remote::Direction::Fetch)
        .into_subrepo_result()?;

    let conn = remote
        .connect(gix::remote::Direction::Fetch)
        .into_subrepo_result()?;

    let should_interrupt = AtomicBool::new(false);
    let _outcome = conn
        .prepare_fetch(
            gix::progress::Discard,
            gix::remote::ref_map::Options {
                prefix_from_spec_as_filter_on_remote: false,
                extra_refspecs: Vec::new(),
                handshake_parameters: Vec::new(),
            },
        )
        .into_subrepo_result()?
        .receive(gix::progress::Discard, &should_interrupt)
        .into_subrepo_result()?;

    let mut fetched_ref = repo.find_reference(dst_ref).into_subrepo_result()?;
    let commit = fetched_ref.peel_to_commit().into_subrepo_result()?;

    let commit_id = commit.id;

    repo.reference(
        dst_ref,
        commit_id,
        gix::refs::transaction::PreviousValue::Any,
        "",
    )
    .into_subrepo_result()?;

    Ok(commit_id)
}
