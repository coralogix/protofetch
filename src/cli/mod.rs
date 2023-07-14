use derive_new::new;

pub mod command_handlers;

#[derive(Clone, Debug, new)]
pub struct HttpGitAuth {
    pub username: String,
    pub password: String,
}
