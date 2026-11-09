// Windows credential store access requires unsafe FFI into the Win32
// security API. Scoped to this module so the rest of the crate stays
// under the workspace-wide `unsafe_code = "warn"` lint.
#![allow(unsafe_code)]

use anyhow::{anyhow, Context, Result};
use std::slice;

use base64::Engine as _;
use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, INVALID_HANDLE_VALUE, TRUE};
use windows_sys::Win32::Security::Credentials::{
    CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
};
use windows_sys::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};

use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::credential::{Account, CredentialStore};
use crate::player::Session;

const USERID_TARGET: &str = "https://www.roblox.com:RobloxStudioAuthuserid";
/// Followed by the user id. Holds the raw `.ROBLOSECURITY` value.
const SESSION_TARGET_PREFIX: &str = "https://www.roblox.com:RobloxStudioAuth.ROBLOSECURITY";
const REGISTRY_PATH: &str =
    r"SOFTWARE\Roblox\RobloxStudio\LoggedInUsersStore\https:\www.roblox.com";
/// The Player's cookie store, under `%LOCALAPPDATA%`. JSON holding
/// `CookiesVersion` and `CookiesData`: the cookies, encrypted with DPAPI for
/// the current Windows user, no extra entropy, then base64.
const PLAYER_COOKIES: &str = r"Roblox\LocalStorage\RobloxCookies.dat";

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

    fn session(&self, user_id: &str) -> Result<Option<Session>> {
        // A missing credential is the normal "listed but never stored" case,
        // so it is `None`; a present one of the wrong shape is an error, since
        // it means Studio changed its storage and the user cannot fix that.
        let Ok(raw) = Self::read_credential(&format!("{SESSION_TARGET_PREFIX}{user_id}")) else {
            return Ok(None);
        };
        Session::parse(&raw).map(Some).ok_or_else(|| {
            anyhow!(
                "The Studio session stored for {user_id} is not in the expected format. \
                 Studio may have changed how it stores sessions."
            )
        })
    }

    fn ensure_player_installed(&self) -> Result<()> {
        winreg::RegKey::predef(winreg::enums::HKEY_CLASSES_ROOT)
            .open_subkey("roblox-player")
            .map(|_| ())
            .map_err(|_| {
                anyhow!("The Roblox Player is not installed: nothing handles roblox-player: links.")
            })
    }

    fn player_running(&self) -> Result<bool> {
        const PLAYER_EXE: &str = "RobloxPlayerBeta.exe";

        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                return Err(anyhow!(
                    "Could not list running processes: {}",
                    std::io::Error::last_os_error()
                ));
            }

            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;

            let mut found = false;
            let mut more = Process32FirstW(snapshot, &mut entry) == TRUE;
            while more {
                let len = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());
                let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
                if name.eq_ignore_ascii_case(PLAYER_EXE) {
                    found = true;
                    break;
                }
                more = Process32NextW(snapshot, &mut entry) == TRUE;
            }

            CloseHandle(snapshot);
            Ok(found)
        }
    }

    fn open_player(&self, uri: &str) -> Result<()> {
        // ShellExecuteW, not `explorer "<uri>"`. explorer is what launches
        // Studio elsewhere in this toolset, but measured on 2026-11-08 it
        // hands the Player a ticket the Player then fails to redeem (HTTP 403
        // on authentication-ticket/redeem, "authentication error" dialog),
        // while the very same ticket opened with ShellExecute signs in. The
        // ticket is only letters, digits, `-` and `_`, so what explorer alters
        // is not known; ShellExecute is what a browser and Start-Process use.
        //
        // The return value is greater than 32 on success, and an error code
        // otherwise.
        let file = to_wide(uri);
        let verb = to_wide("open");
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                file.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        } as isize;
        if result <= 32 {
            return Err(anyhow!(
                "Windows could not open the Roblox Player (ShellExecute error {result})"
            ));
        }
        Ok(())
    }

    fn write_player_session(&self, session: &Session) -> Result<()> {
        let path = dirs::data_local_dir()
            .context("Could not locate %LOCALAPPDATA%")?
            .join(PLAYER_COOKIES);
        let raw = std::fs::read_to_string(&path).with_context(|| {
            format!(
                "Could not read {}. Open the Roblox Player and sign in once, then retry.",
                path.display()
            )
        })?;

        #[derive(serde::Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct Store {
            cookies_version: String,
            cookies_data: String,
        }
        let store: Store = serde_json::from_str(&raw)
            .with_context(|| format!("{} is not in the expected format", path.display()))?;
        let base64 = base64::engine::general_purpose::STANDARD;
        let encrypted = base64
            .decode(store.cookies_data.trim())
            .with_context(|| format!("{} is not in the expected format", path.display()))?;
        let cookies = String::from_utf8(dpapi(&encrypted, false)?)
            .context("The Player's cookie store is not text")?;

        let cookies = crate::player::replace_session(&cookies, session).context(
            "Open the Roblox Player and sign in once, or pass --force to open it signed in",
        )?;
        let data = base64.encode(dpapi(cookies.as_bytes(), true)?);

        // Checked again at the last moment: a Player started since the first
        // check would write its own store back over this one when it exits.
        if self.player_running()? {
            return Err(anyhow!(
                "The Roblox Player was opened during the switch. Close it, then retry."
            ));
        }

        // The keys in the order the Player writes them, through a file beside
        // the store renamed over it, so an interrupted write leaves the old
        // store whole rather than half of a new one.
        let json = format!(
            "{{\"CookiesVersion\":{},\"CookiesData\":{}}}",
            serde_json::to_string(&store.cookies_version)?,
            serde_json::to_string(&data)?,
        );
        let temp = path.with_extension("dat.rbx-account");
        std::fs::write(&temp, json)
            .with_context(|| format!("Could not write {}", temp.display()))?;
        std::fs::rename(&temp, &path)
            .with_context(|| format!("Could not replace {}", path.display()))?;
        Ok(())
    }
}

/// Runs DPAPI one way or the other, for the current Windows user. The two
/// calls share a signature apart from the description argument, which neither
/// direction needs here.
fn dpapi(data: &[u8], protect: bool) -> Result<Vec<u8>> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };

    unsafe {
        let ok = if protect {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        if ok != TRUE {
            return Err(anyhow!(
                "Windows could not {} the Player's cookie store: {}",
                if protect { "encrypt" } else { "decrypt" },
                std::io::Error::last_os_error()
            ));
        }
        let bytes = slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(output.pbData as _);
        Ok(bytes)
    }
}

fn to_wide(s: &str) -> Vec<u16> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    let mut wide: Vec<u16> = OsStr::new(s).encode_wide().collect();
    wide.push(0);
    wide
}
