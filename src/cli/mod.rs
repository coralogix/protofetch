use derive_new::new;

pub mod args;
pub mod command_handlers;

#[derive(Clone, Debug, new)]
pub struct GitAuth {
    pub username: Option<String>,
    pub password: Option<String>,
}
