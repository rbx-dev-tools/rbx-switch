// macOS support: NOT TESTED, based on rbx_cookie patterns
// Requires a Mac with Roblox Studio installed to verify

use anyhow::{anyhow, Result};

use crate::credential::{Account, CredentialStore};
use crate::player::Session;

pub struct MacCredentialStore;

impl MacCredentialStore {
    pub fn new() -> Self {
        Self
    }
}

impl CredentialStore for MacCredentialStore {
    fn list_accounts(&self) -> Result<Vec<Account>> {
        // On macOS, accounts are stored in:
        // defaults read com.Roblox.RobloxStudio "LoggedInUsersStore.https:.www·roblox·com.users"
        // The dots after "www" and "roblox" are special Unicode middle dots (U+00B7)
        //
        // TODO: Implement when Mac access is available
        Err(anyhow!(
            "macOS support is not yet tested. Contributions welcome!"
        ))
    }

    fn current_user_id(&self) -> Result<Option<String>> {
        // On macOS, the userid is stored in:
        // ~/Library/HTTPStorages/com.Roblox.RobloxStudio.binarycookies
        // as a cookie named "/RobloxStudioAuth/userid"
        //
        // TODO: Implement when Mac access is available
        Err(anyhow!(
            "macOS support is not yet tested. Contributions welcome!"
        ))
    }

    fn set_user_id(&self, _user_id: &str) -> Result<()> {
        // TODO: Implement when Mac access is available
        Err(anyhow!(
            "macOS support is not yet tested. Contributions welcome!"
        ))
    }

    fn session(&self, _user_id: &str) -> Result<Option<Session>> {
        Err(anyhow!("--player is Windows only for now."))
    }

    fn ensure_player_installed(&self) -> Result<()> {
        Err(anyhow!("--player is Windows only for now."))
    }

    fn player_running(&self) -> Result<bool> {
        Err(anyhow!("--player is Windows only for now."))
    }

    fn close_player(&self) -> Result<bool> {
        Err(anyhow!("--player is Windows only for now."))
    }

    fn write_player_session(&self, _session: &Session) -> Result<()> {
        Err(anyhow!("--player is Windows only for now."))
    }
}
