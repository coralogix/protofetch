use derive_new::new;
use git2::Config;
use std::env;

pub mod command_handlers;

#[derive(Clone, Debug, new)]
pub struct HttpGitAuth {
    pub username: String,
    pub password: String,
}

impl HttpGitAuth {
    /// Resolve git auth for fetching git repos using https.
    /// Tries to get username and password in the following order.
    /// 1 - From command line arguments (--username, --password)
    /// 2 - From env variables: GIT_USERNAME, GIT_PASSWORD
    /// 3 - From default git config: user.name, user.password
    ///
    /// If 2FA is enabled please generate an access token and use it as password. Please see:
    /// https://github.blog/2013-09-03-two-factor-authentication/#how-does-it-work-for-command-line-git
    pub fn resolve_git_auth(
        config: &Config,
        cli_username: Option<String>,
        cli_password: Option<String>,
    ) -> Option<HttpGitAuth> {
        let username = cli_username
            .or_else(|| env::var("GIT_USERNAME").ok())
            .or_else(|| config.get_string("user.name").ok());
        let password = cli_password
            .or_else(|| env::var("GIT_PASSWORD").ok())
            .or_else(|| config.get_string("user.password").ok());
        match (username, password) {
            (Some(username), Some(password)) => Some(HttpGitAuth { username, password }),
            _ => None,
        }
    }
}
