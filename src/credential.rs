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

    /// Open every running Roblox Player and confirm each one is the Player,
    /// closing none yet. Fails with nothing closed when one cannot be opened
    /// or identified, so a close that would be refused is refused before a
    /// ticket is spent.
    fn open_players(&self) -> Result<Box<dyn OpenPlayers>>;

    /// Read the Player's cookie store and check it can take a session:
    /// present, decryptable, in the expected format and signed in. Changes
    /// nothing, so it runs before a ticket is spent or a Player closed.
    fn open_player_store(&self) -> Result<Box<dyn PlayerStore>>;
}

/// Every running Player, opened and confirmed by
/// [`CredentialStore::open_players`].
pub trait OpenPlayers {
    /// Open and confirm, the same way, any Player started since. Called just
    /// before the close, so a Player opened during the ticket exchange is
    /// closed too rather than left to write its store over the switch. Fails
    /// with nothing closed when a newcomer cannot be opened or identified.
    fn add_newcomers(&mut self) -> Result<()>;

    /// Close them all, game in progress included, and wait for them to exit.
    /// `false` when none was left to close.
    fn close(self: Box<Self>) -> Result<bool>;
}

/// The Player's cookie store, read and checked by
/// [`CredentialStore::open_player_store`].
pub trait PlayerStore {
    /// Keep the store as it was read beside it, before anything irreversible
    /// happens: a backup that cannot be written stops the switch there.
    fn back_up(&self) -> Result<()>;

    /// Put `session` in the closed Player's store, so it opens signed in with
    /// it next time.
    fn write_session(self: Box<Self>, session: &Session) -> Result<()>;
}
