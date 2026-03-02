use crate::error::{Error, Result};

pub fn normalize_subdir(input: &str) -> Result<String> {
    if input.is_empty() {
        return Err(Error::user("subdir not set"));
    }

    if input.starts_with('/') {
        return Err(Error::user(format!(
            "The subdir '{input}' should not be absolute path."
        )));
    }

    if input.len() >= 2 {
        let bytes = input.as_bytes();
        if bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
            return Err(Error::user(format!(
                "The subdir '{input}' should not be absolute path."
            )));
        }
    }

    let mut out = input.to_string();
    if let Some(rest) = out.strip_prefix("./") {
        out = rest.to_owned();
    }

    while out.ends_with('/') {
        out.pop();
    }

    while out.contains("//") {
        out = out.replace("//", "/");
    }

    if out.is_empty() {
        return Err(Error::user("subdir not set"));
    }

    Ok(out)
}

pub fn guess_subdir_from_remote(remote: &str) -> Result<String> {
    let mut dir = remote.trim_end_matches('/').to_string();
    dir = dir.trim_end_matches(".git").to_string();
    dir = dir.trim_end_matches('/').to_string();

    let name = dir.rsplit('/').next().unwrap_or("").to_string();

    if name.is_empty() || name == ".git" {
        return Err(Error::user(format!(
            "Can't determine subdir from '{remote}'."
        )));
    }

    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(Error::user(format!(
            "Can't determine subdir from '{remote}'."
        )));
    }

    normalize_subdir(&name)
}

pub fn encode_subdir(subdir: &str) -> Result<String> {
    let mut subref = subdir.to_string();
    if subref.is_empty() {
        return Ok(subref);
    }

    if is_valid_subrepo_ref(&subref) {
        return Ok(subref);
    }

    subref = subref.replace('%', "%25");

    subref = format!("/{subref}/");
    subref = subref.replace("/.", "/%2e");
    subref = subref.replace(".lock/", "%2elock/");
    subref = subref
        .trim_start_matches('/')
        .trim_end_matches('/')
        .to_string();

    subref = subref.replace("..", "%2e%2e");
    subref = subref.replace("%2e.", "%2e%2e");
    subref = subref.replace(".%2e", "%2e%2e");

    for i in 1u8..32 {
        let needle = char::from(i).to_string();
        let repl = format!("%{i:02x}");
        subref = subref.replace(&needle, &repl);
    }

    subref = subref.replace(char::from(0x7f), "%7f");
    subref = subref.replace(' ', "%20");
    subref = subref.replace('~', "%7e");
    subref = subref.replace('^', "%5e");
    subref = subref.replace(':', "%3a");
    subref = subref.replace('?', "%3f");
    subref = subref.replace('*', "%2a");
    subref = subref.replace('[', "%5b");
    subref = subref.replace('\n', "%0a");

    while subref.contains("//") {
        subref = subref.replace("//", "/");
    }

    if subref.ends_with('.') {
        subref.pop();
        subref.push_str("%2e");
    }

    subref = subref.replace("@{", "%40{");
    subref = subref.replace('\\', "%5c");

    if !is_valid_subrepo_ref(&subref) {
        return Err(Error::user(format!(
            "Can't determine valid subref from '{subdir}'."
        )));
    }

    Ok(subref)
}

fn is_valid_subrepo_ref(subref: &str) -> bool {
    let full = format!("subrepo/{subref}");
    gix::validate::reference::name(full.as_bytes().into()).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_subdir_trims_and_collapses_slashes() {
        assert_eq!(normalize_subdir("./bar///").unwrap(), "bar");
        assert_eq!(normalize_subdir("many////slashes").unwrap(), "many/slashes");
        assert_eq!(normalize_subdir("spa ce/").unwrap(), "spa ce");
    }

    #[test]
    fn normalize_subdir_rejects_absolute_paths() {
        let err = normalize_subdir("/tmp/foo").unwrap_err().to_string();
        assert_eq!(err, "The subdir '/tmp/foo' should not be absolute path.");

        let err = normalize_subdir("C:foo").unwrap_err().to_string();
        assert_eq!(err, "The subdir 'C:foo' should not be absolute path.");
    }

    #[test]
    fn encode_subdir_keeps_valid_ref() {
        assert_eq!(encode_subdir("foo/bar").unwrap(), "foo/bar");
    }

    #[test]
    fn encode_subdir_matches_upstream_examples() {
        assert_eq!(encode_subdir("bar").unwrap(), "bar");
        assert_eq!(encode_subdir(".dot").unwrap(), "%2edot");
        assert_eq!(encode_subdir("end-with.lock").unwrap(), "end-with%2elock");
        assert_eq!(encode_subdir("spa ce").unwrap(), "spa%20ce");
        assert_eq!(encode_subdir("@{").unwrap(), "%40{");
        assert_eq!(encode_subdir("[").unwrap(), "%5b");
        assert_eq!(encode_subdir("back-sl\\as/h").unwrap(), "back-sl%5cas/h");
        assert_eq!(
            encode_subdir("special-char:^[?*").unwrap(),
            "special-char%3a%5e%5b%3f%2a"
        );
        assert_eq!(encode_subdir("many////slashes").unwrap(), "many/slashes");
    }
}
