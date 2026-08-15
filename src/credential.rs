use anyhow::Result;

/// A Roblox account as stored by Studio
#[derive(Debug, Clone)]
pub struct Account {
    pub user_id: String,
    pub username: String,
    #[allow(dead_code)]
    pub profile_pic_url: Option<String>,
}

/// Platform-agnostic trait for reading/writing Studio credentials
pub trait CredentialStore {
    /// List all signed-in accounts
    fn list_accounts(&self) -> Result<Vec<Account>>;

    /// Get the currently active user ID
    fn current_user_id(&self) -> Result<Option<String>>;

    /// Switch the active user ID
    fn set_user_id(&self, user_id: &str) -> Result<()>;
}
