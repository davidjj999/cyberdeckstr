use serde::Deserialize;

#[derive(Deserialize)]
pub struct Config {
    pub npub: Option<String>,
    pub node_address: Option<String>,
    pub node_username: Option<String>,
    pub node_password: Option<String>,
}
