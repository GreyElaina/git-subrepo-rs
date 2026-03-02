#[derive(Debug, Clone)]
pub struct SubrepoRefs {
    pub branch: String,
    pub refs_branch: String,
    pub refs_commit: String,
    pub refs_fetch: String,
    pub refs_push: String,
    pub refs_sync: String,
    pub refs_baseline: String,
}

impl SubrepoRefs {
    pub fn new(subref: &str) -> Self {
        let branch = format!("subrepo/{subref}");
        let prefix = format!("refs/subrepo/{subref}");
        SubrepoRefs {
            branch,
            refs_branch: format!("{prefix}/branch"),
            refs_commit: format!("{prefix}/commit"),
            refs_fetch: format!("{prefix}/fetch"),
            refs_push: format!("{prefix}/push"),
            refs_sync: format!("{prefix}/sync"),
            refs_baseline: format!("{prefix}/baseline"),
        }
    }
}
