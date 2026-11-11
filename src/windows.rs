// Windows credential store access requires unsafe FFI into the Win32
// security API. Scoped to this module so the rest of the crate stays
// under the crate-wide `unsafe_code = "warn"` lint, and every block here
// has to say why it is sound.
#![allow(unsafe_code)]
#![warn(clippy::undocumented_unsafe_blocks)]

use anyhow::{anyhow, bail, Context, Result};
use std::slice;

use base64::Engine as _;
use windows_sys::Win32::Foundation::{
    CloseHandle, LocalFree, ERROR_INVALID_PARAMETER, FILETIME, HANDLE, INVALID_HANDLE_VALUE, TRUE,
    WAIT_OBJECT_0,
};
use windows_sys::Win32::Security::Credentials::{
    CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
};
use windows_sys::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};

use windows_sys::Win32::System::Threading::{
    OpenProcess, TerminateProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
};

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

        // SAFETY: `target_wide` is NUL-terminated and outlives the call, and
        // `p_credential` is a valid place for the out pointer. Nothing is
        // read through that pointer unless the call succeeds.
        let read = unsafe {
            CredReadW(
                target_wide.as_ptr(),
                CRED_TYPE_GENERIC,
                0,
                &mut p_credential,
            )
        };
        if read != TRUE {
            return Err(anyhow!(
                "Failed to read credential: {}",
                std::io::Error::last_os_error()
            ));
        }
        if p_credential.is_null() {
            bail!("Windows reported the credential {target} as read but returned none");
        }
        let credential = Credential(p_credential);

        // SAFETY: the pointer is not null and points at the CREDENTIALW that
        // CredReadW allocated, which stays valid until `credential` frees it
        // at the end of this function, after the last use of `cred`.
        let cred = unsafe { &*credential.0 };
        // SAFETY: Windows documents `CredentialBlob` as pointing at
        // `CredentialBlobSize` bytes inside the same allocation, alive as
        // long as `cred`. `copy_blob` checks the pointer against the length.
        let bytes = unsafe { copy_blob(cred.CredentialBlob, cred.CredentialBlobSize) }?;
        String::from_utf8(bytes).context("Credential value is not valid UTF-8")
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
            // Ignored on write: Windows sets it.
            LastWritten: FILETIME {
                dwLowDateTime: 0,
                dwHighDateTime: 0,
            },
            CredentialBlobSize: value_bytes.len() as u32,
            CredentialBlob: value_bytes.as_ptr() as *mut _,
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            AttributeCount: 0,
            Attributes: std::ptr::null_mut(),
            TargetAlias: std::ptr::null_mut(),
            UserName: username_wide.as_ptr() as *mut _,
        };

        // SAFETY: every pointer in `cred` points into `target_wide`,
        // `value_bytes` or `username_wide`, all alive until this function
        // returns, and the two strings are NUL-terminated. `CredentialBlobSize`
        // is the length of `value_bytes`. CredWriteW only reads them, and
        // copies what it keeps.
        if unsafe { CredWriteW(&cred, 0) } != TRUE {
            return Err(anyhow!(
                "Failed to write credential: {}",
                std::io::Error::last_os_error()
            ));
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
        Ok(!player_pids()?.is_empty())
    }

    fn close_player(&self) -> Result<bool> {
        let mut closed = false;
        for pid in player_pids()? {
            match close_process(pid)? {
                Closed::Terminated => closed = true,
                Closed::AlreadyGone => {}
            }
        }
        Ok(closed)
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

        let cookies = crate::player::replace_session(&cookies, session)
            .context("Open the Roblox Player and sign in once, close it, then retry")?;
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

/// The process ids of every running Roblox Player.
fn player_pids() -> Result<Vec<u32>> {
    const PLAYER_EXE: &str = "RobloxPlayerBeta.exe";

    // SAFETY: plain values in, a handle or INVALID_HANDLE_VALUE out; it reads
    // no memory of ours.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(anyhow!(
            "Could not list running processes: {}",
            std::io::Error::last_os_error()
        ));
    }
    let snapshot = Handle(snapshot);

    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        cntUsage: 0,
        th32ProcessID: 0,
        th32DefaultHeapID: 0,
        th32ModuleID: 0,
        cntThreads: 0,
        th32ParentProcessID: 0,
        pcPriClassBase: 0,
        dwFlags: 0,
        szExeFile: [0; 260],
    };

    let mut pids = Vec::new();
    // SAFETY: `snapshot` is a live process snapshot until its guard closes
    // it, and `entry` is a writable PROCESSENTRY32W whose `dwSize` is set, as
    // the call requires.
    let mut more = unsafe { Process32FirstW(snapshot.0, &mut entry) } == TRUE;
    while more {
        let len = entry
            .szExeFile
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(entry.szExeFile.len());
        let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
        if name.eq_ignore_ascii_case(PLAYER_EXE) {
            pids.push(entry.th32ProcessID);
        }
        // SAFETY: as for Process32FirstW above.
        more = unsafe { Process32NextW(snapshot.0, &mut entry) } == TRUE;
    }

    Ok(pids)
}

/// What became of one Player process `close_process` was asked to close.
#[derive(Debug, PartialEq, Eq)]
enum Closed {
    Terminated,
    /// It exited on its own between the listing and the close.
    AlreadyGone,
}

/// Terminates one Player process and waits for it to exit.
///
/// Terminated rather than asked to close: a Player that exits normally writes
/// its cookie store first, and that write could land after the new session.
fn close_process(pid: u32) -> Result<Closed> {
    // SAFETY: OpenProcess takes plain values and returns a handle or null; it
    // reads no memory of ours.
    let handle = unsafe { OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, 0, pid) };
    if handle.is_null() {
        return not_opened(pid, std::io::Error::last_os_error());
    }
    let handle = Handle(handle);

    // SAFETY: `handle` is open, with the PROCESS_TERMINATE right, until the
    // guard closes it when this function returns.
    if unsafe { TerminateProcess(handle.0, 1) } != TRUE {
        bail!(
            "Could not close the Roblox Player (process {pid}): {}. Close it, then retry.",
            std::io::Error::last_os_error()
        );
    }
    // SAFETY: the same open handle, which carries PROCESS_SYNCHRONIZE.
    if unsafe { WaitForSingleObject(handle.0, 10_000) } != WAIT_OBJECT_0 {
        bail!(
            "The Roblox Player (process {pid}) was told to close but had not exited \
             10 seconds later. Close it, then retry."
        );
    }
    Ok(Closed::Terminated)
}

/// Why OpenProcess failed on a Player process, from the error it left.
///
/// Only a process id that no longer names a process means the Player is gone:
/// OpenProcess answers that with ERROR_INVALID_PARAMETER. Any other failure,
/// such as a Player running as administrator refusing an ordinary user, means
/// it is still running, and the switch must not go on as if it were closed.
fn not_opened(pid: u32, error: std::io::Error) -> Result<Closed> {
    if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
        return Ok(Closed::AlreadyGone);
    }
    bail!(
        "Could not close the Roblox Player (process {pid}): Windows refused to open it \
         ({error}). Close it yourself, then retry."
    )
}

/// A handle this module opened, closed on every path out.
struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle was returned open by Windows, is owned by this
        // guard alone, and is closed here once.
        unsafe { CloseHandle(self.0) };
    }
}

/// A credential CredReadW allocated, freed with CredFree on every path out,
/// errors included.
struct Credential(*mut CREDENTIALW);

impl Drop for Credential {
    fn drop(&mut self) {
        // SAFETY: the pointer is the non-null one a successful CredReadW
        // returned, owned by this guard alone, and freed here once.
        unsafe { CredFree(self.0.cast()) };
    }
}

/// A buffer DPAPI allocated with LocalAlloc, freed with LocalFree on every
/// path out.
struct LocalBuffer(*mut u8);

impl Drop for LocalBuffer {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the pointer is one DPAPI allocated for its output,
            // owned by this guard alone, and freed here once.
            unsafe { LocalFree(self.0.cast()) };
        }
    }
}

/// Copies `len` bytes out of a buffer a Win32 call handed back, refusing a
/// null pointer that claims to hold bytes.
///
/// # Safety
///
/// When `ptr` is not null, it must point at `len` readable bytes.
unsafe fn copy_blob(ptr: *const u8, len: u32) -> Result<Vec<u8>> {
    if len == 0 {
        return Ok(Vec::new());
    }
    if ptr.is_null() {
        bail!("Windows returned {len} bytes at a null address");
    }
    // SAFETY: `ptr` is not null, and the caller guarantees it points at
    // `len` readable bytes.
    Ok(unsafe { slice::from_raw_parts(ptr, len as usize) }.to_vec())
}

/// Runs DPAPI one way or the other, for the current Windows user. The two
/// calls share a signature apart from the description argument, which neither
/// direction needs here.
fn dpapi(data: &[u8], protect: bool) -> Result<Vec<u8>> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(data.len()).context("The Player's cookie store is too large")?,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };

    // SAFETY: `input` points at `data`, `cbData` bytes that outlive the call
    // and that DPAPI only reads. The description, entropy, reserved and
    // prompt arguments are null, which both functions accept, and `output`
    // is a valid place for the result. On success Windows fills `output`
    // with a buffer it allocated, which the guard below frees.
    let ok = unsafe {
        if protect {
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
        }
    };
    if ok != TRUE {
        return Err(anyhow!(
            "Windows could not {} the Player's cookie store: {}",
            if protect { "encrypt" } else { "decrypt" },
            std::io::Error::last_os_error()
        ));
    }
    let buffer = LocalBuffer(output.pbData);
    // SAFETY: on success `pbData` points at `cbData` bytes Windows allocated,
    // alive until `buffer` frees them after this copy. `copy_blob` checks the
    // pointer against the length.
    unsafe { copy_blob(buffer.0, output.cbData) }
}

fn to_wide(s: &str) -> Vec<u16> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    let mut wide: Vec<u16> = OsStr::new(s).encode_wide().collect();
    wide.push(0);
    wide
}

// Nothing below reads the real cookie store or Credential Manager, opens a
// process or talks to Roblox: files live in temp directories, and the one
// DPAPI test encrypts its own data.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_null_blob_that_claims_bytes_is_refused() {
        // SAFETY: a null pointer is the case under test; it is never read.
        let error = unsafe { copy_blob(std::ptr::null(), 4) }.unwrap_err();
        assert!(error.to_string().contains("null"), "{error}");
    }

    #[test]
    fn an_empty_blob_is_empty_whatever_its_pointer() {
        // SAFETY: a length of zero reads nothing.
        assert!(unsafe { copy_blob(std::ptr::null(), 0) }
            .unwrap()
            .is_empty());
    }

    #[test]
    fn a_blob_is_copied_whole() {
        let data = b"1234567890";
        // SAFETY: `data` is 10 readable bytes.
        let copied = unsafe { copy_blob(data.as_ptr(), 10) }.unwrap();
        assert_eq!(copied, data);
    }

    #[test]
    fn dpapi_reads_back_what_it_wrote() {
        let sealed = dpapi(b"test cookies", true).unwrap();
        assert_ne!(sealed, b"test cookies");
        assert_eq!(dpapi(&sealed, false).unwrap(), b"test cookies");
    }

    #[test]
    fn dpapi_refuses_data_it_did_not_write() {
        assert!(dpapi(b"not encrypted", false).is_err());
    }

    #[test]
    fn registry_entries_are_split_and_read() {
        let users = r#"{"1234567890":{"username":"MainAccount","profilePicUrl":"https://example.com/a.png"}};{"9876543210":{"username":"DevAccount"}};"#;
        let accounts = WindowsCredentialStore::parse_accounts_from_registry(users).unwrap();
        let ids: Vec<_> = accounts.iter().map(|a| a.user_id.as_str()).collect();
        assert_eq!(ids, ["1234567890", "9876543210"]);
        assert_eq!(accounts[0].username, "MainAccount");
        assert_eq!(
            accounts[0].profile_pic_url.as_deref(),
            Some("https://example.com/a.png")
        );
        assert_eq!(accounts[1].profile_pic_url, None);
    }

    #[test]
    fn a_player_that_exited_before_the_close_counts_as_gone() {
        let error = std::io::Error::from_raw_os_error(ERROR_INVALID_PARAMETER as i32);
        assert_eq!(not_opened(1234, error).unwrap(), Closed::AlreadyGone);
    }

    /// The message Windows gives is localised, so only the process id and
    /// this tool's own words are checked.
    #[test]
    fn a_player_windows_refuses_to_open_is_not_closed() {
        const ERROR_ACCESS_DENIED: i32 = 5;
        let error = std::io::Error::from_raw_os_error(ERROR_ACCESS_DENIED);
        let error = not_opened(1234, error).unwrap_err().to_string();
        assert!(error.contains("process 1234"), "{error}");
        assert!(error.contains("refused to open it"), "{error}");
    }

    #[test]
    fn a_malformed_registry_entry_is_an_error() {
        assert!(WindowsCredentialStore::parse_accounts_from_registry("{not json").is_err());
    }
}
