use anyhow::Result;

use crate::player::Session;

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

    /// The Studio session stored for one account, `None` when Studio lists
    /// the account but holds no session for it.
    fn session(&self, user_id: &str) -> Result<Option<Session>>;

    /// Fail early, before a ticket is spent, when nothing on this machine
    /// handles `roblox-player:` links.
    fn ensure_player_installed(&self) -> Result<()>;

    /// Whether a Roblox Player process is running right now.
    fn player_running(&self) -> Result<bool>;

    /// Close every running Roblox Player, game in progress included, and wait
    /// for them to exit. `false` when none was running.
    fn close_player(&self) -> Result<bool>;

    /// Read the Player's cookie store and check it can take a session:
    /// present, decryptable, in the expected format and signed in. Changes
    /// nothing, so it runs before a ticket is spent or a Player closed.
    fn open_player_store(&self) -> Result<Box<dyn PlayerStore>>;
}

/// The Player's cookie store, read and checked by
/// [`CredentialStore::open_player_store`].
pub trait PlayerStore {
    /// Put `session` in the closed Player's store, so it opens signed in with
    /// it next time.
    fn write_session(self: Box<Self>, session: &Session) -> Result<()>;
}
