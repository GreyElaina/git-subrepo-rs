use crate::error::{Error, Result, SubrepoResultExt};
use gix_config::File;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinMethod {
    Merge,
    Rebase,
}

impl JoinMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            JoinMethod::Merge => "merge",
            JoinMethod::Rebase => "rebase",
        }
    }
}

impl std::str::FromStr for JoinMethod {
    type Err = Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "merge" => Ok(JoinMethod::Merge),
            "rebase" => Ok(JoinMethod::Rebase),
            other => Err(Error::user(format!("Invalid method '{other}'"))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitRepoState {
    pub remote: String,
    pub branch: String,
    pub commit: String,
    pub parent: String,
    pub method: JoinMethod,
    pub cmdver: String,
}

impl GitRepoState {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let bstr: &gix::bstr::BStr = bytes.into();
        let cfg = File::try_from(bstr).into_subrepo_result()?;

        let remote = cfg
            .raw_value("subrepo.remote")
            .map_err(|_| Error::user("Missing subrepo.remote"))?
            .to_string();
        let branch = cfg
            .raw_value("subrepo.branch")
            .map_err(|_| Error::user("Missing subrepo.branch"))?
            .to_string();
        let commit = cfg
            .raw_value("subrepo.commit")
            .map_err(|_| Error::user("Missing subrepo.commit"))?
            .to_string();
        let parent = cfg
            .raw_value("subrepo.parent")
            .map_err(|_| Error::user("Missing subrepo.parent"))?
            .to_string();

        let method = cfg
            .raw_value("subrepo.method")
            .map_err(|_| Error::user("Missing subrepo.method"))?
            .to_string();
        let method: JoinMethod = method.parse()?;

        let cmdver = cfg
            .raw_value("subrepo.cmdver")
            .map_err(|_| Error::user("Missing subrepo.cmdver"))?
            .to_string();

        Ok(GitRepoState {
            remote,
            branch,
            commit,
            parent,
            method,
            cmdver,
        })
    }

    pub fn format(&self) -> String {
        format!(
            "{header}\n\
[subrepo]\n\
    remote = {remote}\n\
    branch = {branch}\n\
    commit = {commit}\n\
    parent = {parent}\n\
    method = {method}\n\
    cmdver = {cmdver}\n",
            header = comment_header(),
            remote = self.remote,
            branch = self.branch,
            commit = self.commit,
            parent = self.parent,
            method = self.method.as_str(),
            cmdver = self.cmdver,
        )
    }
}

pub fn comment_header() -> &'static str {
    "; DO NOT EDIT (unless you know what you are doing)\n\
;\n\
; This subdirectory is a git \"subrepo\", and this file is maintained by the\n\
; git-subrepo command. See https://github.com/ingydotnet/git-subrepo#readme\n\
;"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_includes_required_comment_block() {
        let state = GitRepoState {
            remote: "r".to_string(),
            branch: "master".to_string(),
            commit: "c".to_string(),
            parent: "p".to_string(),
            method: JoinMethod::Merge,
            cmdver: "0.0.0".to_string(),
        };

        let formatted = state.format();
        let expected_prefix = format!("{}\n[subrepo]\n", comment_header());
        assert!(
            formatted.starts_with(&expected_prefix),
            "formatted header mismatch:\n{formatted}"
        );
    }
}
