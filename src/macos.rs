// macOS support — NOT TESTED, based on rbx_cookie patterns
// Requires a Mac with Roblox Studio installed to verify

use anyhow::{anyhow, Result};

use crate::credential::{Account, CredentialStore};

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
            "macOS support is not yet tested — contributions welcome!"
        ))
    }

    fn current_user_id(&self) -> Result<Option<String>> {
        // On macOS, the userid is stored in:
        // ~/Library/HTTPStorages/com.Roblox.RobloxStudio.binarycookies
        // as a cookie named "/RobloxStudioAuth/userid"
        //
        // TODO: Implement when Mac access is available
        Err(anyhow!(
            "macOS support is not yet tested — contributions welcome!"
        ))
    }

    fn set_user_id(&self, _user_id: &str) -> Result<()> {
        // TODO: Implement when Mac access is available
        Err(anyhow!(
            "macOS support is not yet tested — contributions welcome!"
        ))
    }
}
