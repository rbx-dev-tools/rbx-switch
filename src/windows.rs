// Windows credential store access requires unsafe FFI into the Win32
// security API. Scoped to this module so the rest of the crate stays
// under the workspace-wide `unsafe_code = "warn"` lint.
#![allow(unsafe_code)]

use anyhow::{anyhow, Context, Result};
use std::slice;

use windows_sys::Win32::Foundation::TRUE;
use windows_sys::Win32::Security::Credentials::{
    CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
};

use crate::credential::{Account, CredentialStore};

const USERID_TARGET: &str = "https://www.roblox.com:RobloxStudioAuthuserid";
const REGISTRY_PATH: &str =
    r"SOFTWARE\Roblox\RobloxStudio\LoggedInUsersStore\https:\www.roblox.com";

pub struct WindowsCredentialStore;

impl WindowsCredentialStore {
    pub fn new() -> Self {
        Self
    }

    fn read_credential(target: &str) -> Result<String> {
        let target_wide = to_wide(target);
        let mut p_credential: *mut CREDENTIALW = std::ptr::null_mut();

        unsafe {
            if CredReadW(
                target_wide.as_ptr(),
                CRED_TYPE_GENERIC,
                0,
                &mut p_credential,
            ) != TRUE
            {
                return Err(anyhow!(
                    "Failed to read credential: {}",
                    std::io::Error::last_os_error()
                ));
            }

            let cred = &*p_credential;
            let bytes =
                slice::from_raw_parts(cred.CredentialBlob, cred.CredentialBlobSize as usize);
            let value =
                String::from_utf8(bytes.to_vec()).context("Credential value is not valid UTF-8")?;

            CredFree(p_credential as *mut _);
            Ok(value)
        }
    }

    fn write_credential(target: &str, value: &str) -> Result<()> {
        let target_wide = to_wide(target);
        let value_bytes = value.as_bytes().to_vec();
        let username_wide = to_wide("");

        let cred = CREDENTIALW {
            Flags: 0,
            Type: CRED_TYPE_GENERIC,
            TargetName: target_wide.as_ptr() as *mut _,
            Comment: std::ptr::null_mut(),
            LastWritten: unsafe { std::mem::zeroed() },
            CredentialBlobSize: value_bytes.len() as u32,
            CredentialBlob: value_bytes.as_ptr() as *mut _,
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            AttributeCount: 0,
            Attributes: std::ptr::null_mut(),
            TargetAlias: std::ptr::null_mut(),
            UserName: username_wide.as_ptr() as *mut _,
        };

        unsafe {
            if CredWriteW(&cred, 0) != TRUE {
                return Err(anyhow!(
                    "Failed to write credential: {}",
                    std::io::Error::last_os_error()
                ));
            }
        }

        Ok(())
    }

    fn parse_accounts_from_registry(json_str: &str) -> Result<Vec<Account>> {
        let mut accounts = Vec::new();

        // Format: {"userId":{"username":"name","profilePicUrl":"url"}};{"userId2":{...}};...
        for entry in json_str.split(';') {
            let entry = entry.trim();
            if entry.is_empty() {
                continue;
            }

            let parsed: serde_json::Value =
                serde_json::from_str(entry).context("Failed to parse account entry")?;

            if let Some(obj) = parsed.as_object() {
                for (user_id, data) in obj {
                    let username = data
                        .get("username")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();

                    let profile_pic_url = data
                        .get("profilePicUrl")
                        .and_then(|v| v.as_str())
                        .map(String::from);

                    accounts.push(Account {
                        user_id: user_id.clone(),
                        username,
                        profile_pic_url,
                    });
                }
            }
        }

        Ok(accounts)
    }
}

impl CredentialStore for WindowsCredentialStore {
    fn list_accounts(&self) -> Result<Vec<Account>> {
        let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
        let key = hkcu.open_subkey(REGISTRY_PATH).context(
            "Could not open LoggedInUsersStore registry key. Is Roblox Studio installed?",
        )?;

        let users_json: String = key
            .get_value("users")
            .context("No 'users' value found in LoggedInUsersStore")?;

        Self::parse_accounts_from_registry(&users_json)
    }

    fn current_user_id(&self) -> Result<Option<String>> {
        match Self::read_credential(USERID_TARGET) {
            Ok(id) => Ok(Some(id)),
            Err(_) => Ok(None),
        }
    }

    fn set_user_id(&self, user_id: &str) -> Result<()> {
        Self::write_credential(USERID_TARGET, user_id)
    }
}

fn to_wide(s: &str) -> Vec<u16> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    let mut wide: Vec<u16> = OsStr::new(s).encode_wide().collect();
    wide.push(0);
    wide
}
