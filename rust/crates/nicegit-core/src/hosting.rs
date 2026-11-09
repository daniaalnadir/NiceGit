//! Web addresses for repositories on GitHub, GitLab, and Bitbucket: a repository's home page, its
//! commits, branches, files, and pull or merge requests. A remote address is read into its parts
//! and every link is built from those parts alone, so a credential or port in the remote never
//! appears in a link. Nothing here calls the network or uses a token.

use crate::models::{GitError, Result};

/// The hosting services NiceGit makes links for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Host {
    GitHub,
    GitLab,
    Bitbucket,
}

impl Host {
    /// The service's name as shown to the user.
    pub fn name(self) -> &'static str {
        match self {
            Host::GitHub => "GitHub",
            Host::GitLab => "GitLab",
            Host::Bitbucket => "Bitbucket",
        }
    }
}

/// The self-hosted hosts listed in Settings, for each service. github.com, gitlab.com, and
/// bitbucket.org are always supported, so they are not listed.
#[derive(Clone, Copy, Debug, Default)]
pub struct ListedHosts<'a> {
    pub github: &'a [String],
    pub gitlab: &'a [String],
}

/// A repository on a supported host, read from its remote address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostedRepository {
    /// The lower-cased host name, such as `gitlab.com` or `git.example.com`.
    pub host: String,
    pub kind: Host,
    /// The owner, or for GitLab the group path such as `group/subgroup`, without the repository.
    pub owner_path: String,
    /// The repository's name, without a `.git` suffix.
    pub name: String,
}

/// Splits a remote address into its lower-cased host and its path. Accepts `https://` and `ssh://`
/// URLs and scp-like `[user@]host:path` addresses. A user name is dropped; a password, port, query,
/// or fragment is refused, as is any other scheme. `None` for an address that does not parse.
pub fn split_address(address: &str) -> Option<(String, &str)> {
    let address = address.trim();
    let (host, path) = if let Some((scheme, rest)) = address.split_once("://") {
        if !(scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("ssh")) {
            return None;
        }
        let (authority, path) = match rest.find('/') {
            Some(index) => (&rest[..index], &rest[index..]),
            None => (rest, ""),
        };
        if path.contains('?') || path.contains('#') || authority.contains('?') || authority.contains('#') {
            return None;
        }
        let host = match authority.rsplit_once('@') {
            Some((userinfo, host)) => {
                // A user name is allowed; a password is not.
                if userinfo.contains(':') {
                    return None;
                }
                host
            }
            None => authority,
        };
        (host, path)
    } else {
        // scp-like form: [user@]host:path. A local path such as /srv/repo has no host match.
        let (host_part, path) = address.split_once(':')?;
        let host = host_part.rsplit_once('@').map(|(_, host)| host).unwrap_or(host_part);
        (host, path)
    };
    // A port, or any other colon, is refused before the host is compared.
    if host.is_empty() || host.contains(':') {
        return None;
    }
    Some((host.to_ascii_lowercase(), path))
}

/// Which service a host name belongs to. The fixed hosts come first, then the Settings lists.
fn classify(host: &str, hosts: ListedHosts) -> Option<Host> {
    let listed = |list: &[String]| list.iter().any(|item| item.eq_ignore_ascii_case(host));
    match host {
        "github.com" => Some(Host::GitHub),
        "gitlab.com" => Some(Host::GitLab),
        "bitbucket.org" => Some(Host::Bitbucket),
        _ if listed(hosts.github) => Some(Host::GitHub),
        _ if listed(hosts.gitlab) => Some(Host::GitLab),
        _ => None,
    }
}

/// Whether `part` can be a path segment on these hosts: letters, digits, `-`, `_`, and `.`.
fn valid_segment(part: &str) -> bool {
    !part.is_empty() && part != "." && part != ".." && part.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Percent-encodes a commit, branch, or path for a URL, keeping its slashes as separators.
pub fn encode_path(text: &str) -> String {
    let mut encoded = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => encoded.push(byte as char),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

impl HostedRepository {
    /// Reads the repository from a remote address on GitHub, GitLab, or Bitbucket. GitHub and
    /// Bitbucket repositories are `owner/name`; GitLab repositories may sit in nested groups.
    pub fn parse(address: &str, hosts: ListedHosts) -> Result<Self> {
        let unsupported = || GitError::failed("Hosting", "This remote is not a supported repository address.");
        let (host, path) = split_address(address).ok_or_else(unsupported)?;
        let Some(kind) = classify(&host, hosts) else {
            return Err(GitError::failed(
                "Hosting",
                "This remote is not on GitHub, GitLab, or Bitbucket, or a GitHub or GitLab host listed in Settings.",
            ));
        };
        let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
        let Some((last, owner)) = parts.split_last() else {
            return Err(GitError::failed("Hosting", "The remote must identify an owner and repository."));
        };
        let name = last.strip_suffix(".git").unwrap_or(*last);
        let owner_ok = match kind {
            Host::GitLab => !owner.is_empty(),
            Host::GitHub | Host::Bitbucket => owner.len() == 1,
        };
        if !owner_ok {
            return Err(GitError::failed("Hosting", "The remote must identify an owner and repository."));
        }
        if !valid_segment(name) || !owner.iter().all(|part| valid_segment(part)) {
            return Err(GitError::failed("Hosting", "The repository address is invalid."));
        }
        Ok(Self { host, kind, owner_path: owner.join("/"), name: name.to_string() })
    }

    /// The owner and name, such as `acme/app`, or `group/subgroup/app` on GitLab.
    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner_path, self.name)
    }

    /// The address of the site, such as `https://git.example.com`.
    pub fn web_base(&self) -> String {
        format!("https://{}", self.host)
    }

    /// The repository's home page.
    pub fn home_url(&self) -> String {
        format!("{}/{}", self.web_base(), self.slug())
    }

    /// The repository's pull requests (GitHub, Bitbucket) or merge requests (GitLab) list.
    pub fn pull_requests_url(&self) -> String {
        format!("{}/{}", self.home_url(), self.route("pulls", "-/merge_requests", "pull-requests"))
    }

    /// A commit's page. It exists only once the commit is on the remote.
    pub fn commit_url(&self, commit: &str) -> String {
        format!("{}/{}{}", self.home_url(), self.route("commit/", "-/commit/", "commits/"), encode_path(commit))
    }

    /// A branch's page, with its slashes kept.
    pub fn branch_url(&self, branch: &str) -> String {
        format!("{}/{}{}", self.home_url(), self.route("tree/", "-/tree/", "branch/"), encode_path(branch))
    }

    /// A file's page as it was in `commit`, with the path's slashes kept.
    pub fn file_url(&self, commit: &str, path: &str) -> String {
        format!("{}/{}{}/{}", self.home_url(), self.route("blob/", "-/blob/", "src/"), encode_path(commit), encode_path(path))
    }

    /// Another page of a GitHub repository, such as `issues` or `actions`. Other hosts have no
    /// such page here, so they give `None`.
    pub fn github_page_url(&self, page: &str) -> Option<String> {
        (self.kind == Host::GitHub).then(|| format!("{}/{page}", self.home_url()))
    }

    /// The path segment for this host's service: `github`, `gitlab`, or `bitbucket`.
    fn route<'a>(&self, github: &'a str, gitlab: &'a str, bitbucket: &'a str) -> &'a str {
        match self.kind {
            Host::GitHub => github,
            Host::GitLab => gitlab,
            Host::Bitbucket => bitbucket,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{encode_path, split_address, HostedRepository, ListedHosts};
    use crate::hosting::Host;

    fn hosts(list: &[&str]) -> Vec<String> {
        list.iter().map(|host| host.to_string()).collect()
    }

    fn parse(address: &str) -> Option<HostedRepository> {
        HostedRepository::parse(address, ListedHosts::default()).ok()
    }

    #[test]
    fn gitlab_addresses_keep_nested_groups_in_every_form() {
        let expected =
            HostedRepository { host: "gitlab.com".into(), kind: Host::GitLab, owner_path: "group/sub".into(), name: "proj".into() };
        for address in [
            "https://gitlab.com/group/sub/proj.git",
            "https://gitlab.com/group/sub/proj",
            "https://GitLab.com/group/sub/proj/",
            "ssh://git@gitlab.com/group/sub/proj.git",
            "git@gitlab.com:group/sub/proj.git",
            "gitlab.com:group/sub/proj",
        ] {
            assert_eq!(parse(address), Some(expected.clone()), "{address}");
        }
    }

    #[test]
    fn bitbucket_and_github_addresses_take_exactly_an_owner_and_a_name() {
        let bitbucket = parse("git@bitbucket.org:team/tool.git").unwrap();
        assert_eq!((bitbucket.kind, bitbucket.owner_path.as_str(), bitbucket.name.as_str()), (Host::Bitbucket, "team", "tool"));
        assert_eq!(parse("https://bitbucket.org/team/tool").unwrap(), bitbucket);
        assert_eq!(parse("https://bitbucket.org/team/tool/sub"), None, "Bitbucket has no nested workspaces");
        assert_eq!(parse("https://github.com/a/b/c"), None, "GitHub has no nested owners");
        assert_eq!(parse("https://github.com/owner"), None, "a name is needed");
        assert_eq!(parse("https://gitlab.com/"), None, "a path is needed");
    }

    #[test]
    fn self_hosted_gitlab_hosts_need_to_be_listed() {
        let listed = hosts(&["Git.Example.com"]);
        let hosts = ListedHosts { github: &[], gitlab: &listed };
        let repository = HostedRepository::parse("https://octo@GIT.example.com/acme/tools/app.git", hosts).unwrap();
        assert_eq!((repository.kind, repository.host.as_str()), (Host::GitLab, "git.example.com"));
        assert_eq!(repository.owner_path, "acme/tools");
        assert_eq!(parse("https://git.example.com/acme/app"), None, "not listed");
    }

    #[test]
    fn a_listed_github_host_is_github_and_fixed_hosts_win() {
        let github = hosts(&["git.example.com"]);
        let gitlab = hosts(&["gitlab.example.com"]);
        let hosts = ListedHosts { github: &github, gitlab: &gitlab };
        let enterprise = HostedRepository::parse("https://git.example.com/acme/app", hosts).unwrap();
        assert_eq!(enterprise.kind, Host::GitHub);
        let self_hosted = HostedRepository::parse("ssh://git@gitlab.example.com/acme/app.git", hosts).unwrap();
        assert_eq!(self_hosted.kind, Host::GitLab);
        assert!(HostedRepository::parse("https://gitlab.example.com/a/b/c", ListedHosts { github: &github, gitlab: &[] }).is_err());
    }

    #[test]
    fn ports_credentials_queries_and_other_schemes_are_refused_or_stripped() {
        assert_eq!(parse("ssh://git@gitlab.com:2222/group/proj.git"), None, "a port");
        assert_eq!(parse("https://gitlab.com:443/group/proj"), None, "a port");
        assert_eq!(parse("https://user:secret@gitlab.com/group/proj"), None, "a password");
        assert_eq!(parse("https://gitlab.com/group/proj?tab=1"), None, "a query");
        assert_eq!(parse("https://gitlab.com/group/proj#top"), None, "a fragment");
        assert_eq!(parse("http://gitlab.com/group/proj"), None, "plain http");
        assert_eq!(parse("file:///srv/gitlab.com/group/proj"), None, "a file address");
        let user_only = parse("https://octo@gitlab.com/group/proj").unwrap();
        assert_eq!(user_only.host, "gitlab.com", "a user name is dropped");
        assert!(!user_only.home_url().contains("octo"));
    }

    #[test]
    fn unknown_hosts_and_bad_names_are_refused() {
        assert_eq!(parse("https://example.com/group/proj"), None);
        assert_eq!(parse("/srv/archive/proj.git"), None, "a local path");
        assert_eq!(parse("https://gitlab.com/group/../proj"), None, "a dot segment");
        assert_eq!(parse("https://gitlab.com/group/%20proj"), None, "an encoded name");
        assert_eq!(parse("https://gitlab.com/group/.git"), None, "an empty name");
    }

    #[test]
    fn gitlab_urls_use_the_dash_routes() {
        let repository = parse("https://gitlab.com/group/sub/proj.git").unwrap();
        assert_eq!(repository.home_url(), "https://gitlab.com/group/sub/proj");
        assert_eq!(repository.pull_requests_url(), "https://gitlab.com/group/sub/proj/-/merge_requests");
        assert_eq!(repository.commit_url("abc123"), "https://gitlab.com/group/sub/proj/-/commit/abc123");
        assert_eq!(repository.branch_url("feature/x"), "https://gitlab.com/group/sub/proj/-/tree/feature/x");
        assert_eq!(repository.file_url("abc123", "dir/a b.txt"), "https://gitlab.com/group/sub/proj/-/blob/abc123/dir/a%20b.txt");
        assert_eq!(repository.github_page_url("issues"), None, "GitLab has no GitHub pages");
    }

    #[test]
    fn bitbucket_urls_use_its_own_routes() {
        let repository = parse("https://bitbucket.org/team/tool.git").unwrap();
        assert_eq!(repository.home_url(), "https://bitbucket.org/team/tool");
        assert_eq!(repository.pull_requests_url(), "https://bitbucket.org/team/tool/pull-requests");
        assert_eq!(repository.commit_url("abc123"), "https://bitbucket.org/team/tool/commits/abc123");
        assert_eq!(repository.branch_url("feature/x"), "https://bitbucket.org/team/tool/branch/feature/x");
        assert_eq!(repository.file_url("abc123", "src/main.rs"), "https://bitbucket.org/team/tool/src/abc123/src/main.rs");
    }

    #[test]
    fn github_urls_keep_their_existing_routes() {
        let repository = parse("git@github.com:owner/repo.git").unwrap();
        assert_eq!(repository.home_url(), "https://github.com/owner/repo");
        assert_eq!(repository.pull_requests_url(), "https://github.com/owner/repo/pulls");
        assert_eq!(repository.commit_url("abc123"), "https://github.com/owner/repo/commit/abc123");
        assert_eq!(repository.branch_url("feature/x"), "https://github.com/owner/repo/tree/feature/x");
        assert_eq!(repository.file_url("abc123", "dir/a b#c.txt"), "https://github.com/owner/repo/blob/abc123/dir/a%20b%23c.txt");
        assert_eq!(repository.github_page_url("actions").as_deref(), Some("https://github.com/owner/repo/actions"));
    }

    #[test]
    fn branch_names_encode_reserved_characters_but_keep_slashes() {
        let repository = parse("https://gitlab.com/group/proj").unwrap();
        assert_eq!(repository.branch_url("feat #1?/a+b%"), "https://gitlab.com/group/proj/-/tree/feat%20%231%3F/a%2Bb%25");
        assert_eq!(encode_path("ü?.txt"), "%C3%BC%3F.txt");
    }

    #[test]
    fn split_address_reports_the_host_in_lower_case_and_the_path() {
        assert_eq!(split_address("  SSH://Me@Host.Example/a/b  "), Some(("host.example".to_string(), "/a/b")));
        assert_eq!(split_address("me@host:a/b"), Some(("host".to_string(), "a/b")));
        assert_eq!(split_address("/srv/a:b"), Some(("/srv/a".to_string(), "b")), "a colon in a path reads as a host");
        assert_eq!(split_address("/srv/repo"), None);
    }
}
