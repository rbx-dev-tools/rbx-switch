// Windows credential store access requires unsafe FFI into the Win32
// security API. Scoped to this module so the rest of the crate stays
// under the crate-wide `unsafe_code = "warn"` lint, and every block here
// has to say why it is sound.
#![allow(unsafe_code)]
#![warn(clippy::undocumented_unsafe_blocks)]

use anyhow::{anyhow, bail, Context, Result};
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::slice;
use std::sync::atomic::{AtomicU32, Ordering};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use colored::Colorize as _;
use windows_sys::Win32::Foundation::{
    CloseHandle, LocalFree, ERROR_INSUFFICIENT_BUFFER, ERROR_INVALID_PARAMETER, FILETIME, HANDLE,
    INVALID_HANDLE_VALUE, TRUE, WAIT_OBJECT_0,
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
    OpenProcess, QueryFullProcessImageNameW, TerminateProcess, WaitForSingleObject,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
};

use crate::credential::{Account, CredentialStore, OpenPlayers, PlayerStore};
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

    fn open_players(&self) -> Result<Box<dyn OpenPlayers>> {
        let opened = player_pids()?
            .into_iter()
            .map(|pid| (pid, open_process(pid)))
            .collect();
        Ok(Box::new(Players(confirm_players(opened)?)))
    }

    fn open_player_store(&self) -> Result<Box<dyn PlayerStore>> {
        Ok(Box::new(OpenStore::open(StoreAccess::player()?)?))
    }
}

impl PlayerStore for OpenStore {
    fn back_up(&self) -> Result<()> {
        OpenStore::back_up(self)
    }

    fn write_session(self: Box<Self>, session: &Session) -> Result<()> {
        self.write(session)
    }
}

/// What to say when the store holds no session to replace.
const SIGN_IN_ONCE: &str = "Open the Roblox Player and sign in once, close it, then retry";

/// Beside the store: the previous store, kept one switch deep.
const BACKUP_SUFFIX: &str = ".rbx-account.bak";

/// The Player's cookie store file: the two keys this tool reads, in the order
/// the Player writes them, and every other key kept as it was, so a key a
/// later Player adds survives a switch.
#[derive(Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "PascalCase")]
struct StoreFile {
    cookies_version: String,
    cookies_data: String,
    #[serde(flatten)]
    other: serde_json::Map<String, serde_json::Value>,
}

/// Where a cookie store lives, how its cookies are sealed, and how to tell
/// whether a Player is open. The real one is the Player's file under DPAPI;
/// the tests point one at a temp file, with a stand-in cipher.
struct StoreAccess {
    path: PathBuf,
    seal: fn(&[u8]) -> Result<Vec<u8>>,
    unseal: fn(&[u8]) -> Result<Vec<u8>>,
    player_open: fn() -> Result<bool>,
}

impl StoreAccess {
    fn player() -> Result<Self> {
        Ok(Self {
            path: dirs::data_local_dir()
                .context("Could not locate %LOCALAPPDATA%")?
                .join(PLAYER_COOKIES),
            seal: |data| dpapi(data, true),
            unseal: |data| dpapi(data, false),
            player_open: || Ok(!player_pids()?.is_empty()),
        })
    }
}

/// A cookie store read, decrypted and checked to hold a session, with
/// nothing changed yet.
struct OpenStore {
    access: StoreAccess,
    /// The file as read, to tell whether it changed before the write.
    raw: String,
    file: StoreFile,
    cookies: String,
}

impl OpenStore {
    fn open(access: StoreAccess) -> Result<Self> {
        let raw = fs::read_to_string(&access.path).with_context(|| {
            format!(
                "Could not read {}. Open the Roblox Player and sign in once, then retry.",
                access.path.display()
            )
        })?;
        let (file, cookies) = parse_store(&access, &raw)?;
        Ok(Self {
            access,
            raw,
            file,
            cookies,
        })
    }

    /// Keeps the store as it was read beside it, replacing the backup of the
    /// switch before. One deep: it holds the store as it was before the
    /// latest switch.
    fn back_up(&self) -> Result<()> {
        let backup = beside(&self.access.path, BACKUP_SUFFIX);
        replace_synced(&backup, self.raw.as_bytes())
            .with_context(|| format!("Could not keep a backup at {}", backup.display()))
    }

    /// Puts `session` in the store, every other cookie and key kept.
    fn write(self, session: &Session) -> Result<()> {
        let Self {
            access,
            raw,
            file,
            cookies,
        } = self;

        // Read again: the store was checked before the ticket was fetched and
        // the Player closed, and a Player may have rewritten it since. The
        // store on disk now is the one that takes the session.
        let current = fs::read_to_string(&access.path)
            .with_context(|| format!("Could not read {}", access.path.display()))?;
        let (file, cookies) = if current == raw {
            (file, cookies)
        } else {
            parse_store(&access, &current)?
        };

        let cookies = crate::player::replace_session(&cookies, session).context(SIGN_IN_ONCE)?;
        let sealed = (access.seal)(cookies.as_bytes())?;
        let json = serde_json::to_string(&StoreFile {
            cookies_data: BASE64.encode(sealed),
            ..file
        })?;

        // Checked again at the last moment: a Player started since the first
        // check would write its own store back over this one when it exits.
        if (access.player_open)()? {
            bail!("The Roblox Player was opened during the switch. Close it, then retry.");
        }

        // Through a temp file renamed over the store, so an interrupted write
        // leaves the old store whole rather than half of a new one.
        replace_synced(&access.path, json.as_bytes())
    }
}

/// Reads a store file down to its cookies, refusing anything this tool could
/// not write a session into.
fn parse_store(access: &StoreAccess, raw: &str) -> Result<(StoreFile, String)> {
    let path = access.path.display();
    let file: StoreFile = serde_json::from_str(raw)
        .with_context(|| format!("{path} is not in the expected format"))?;
    let sealed = BASE64
        .decode(file.cookies_data.trim())
        .with_context(|| format!("{path} is not in the expected format"))?;
    let cookies = String::from_utf8((access.unseal)(&sealed)?)
        .context("The Player's cookie store is not text")?;
    if !crate::player::holds_session(&cookies) {
        bail!(
            "The Roblox Player is signed out, so it holds no session to replace. {SIGN_IN_ONCE}."
        );
    }
    Ok((file, cookies))
}

/// Replaces `path` with `contents` through a temp file of its own in the same
/// directory, flushed to disk before the rename.
fn replace_synced(path: &Path, contents: &[u8]) -> Result<()> {
    let temp = temp_beside(path);
    write_new_synced(&temp, contents)
        .with_context(|| format!("Could not write {}", temp.display()))?;
    fs::rename(&temp, path).map_err(|error| {
        // Best effort: the rename error is the one worth reading.
        let _ = fs::remove_file(&temp);
        anyhow!(error).context(format!("Could not replace {}", path.display()))
    })
}

/// Tells this process's temp files apart from one another.
static TEMP_COUNTER: AtomicU32 = AtomicU32::new(0);

/// A temp file name beside `path` that no other write uses: this process's
/// id and a counter. Two runs at once each write their own file, so neither
/// can rename the other's half-written one over the store.
fn temp_beside(path: &Path) -> PathBuf {
    let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    beside(
        path,
        &format!(".rbx-account.{}.{n}.tmp", std::process::id()),
    )
}

/// Writes a file that must not exist yet, shared with nobody while it is
/// open, and flushes it to disk. A file this call created and could not
/// finish is removed.
fn write_new_synced(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    use std::os::windows::fs::OpenOptionsExt as _;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .share_mode(0)
        .open(path)?;
    let written = file.write_all(contents).and_then(|()| file.sync_all());
    if written.is_err() {
        drop(file);
        let _ = fs::remove_file(path);
    }
    written
}

/// `path` with `suffix` added to its file name.
fn beside(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

/// The Roblox Player's executable.
const PLAYER_EXE: &str = "RobloxPlayerBeta.exe";

/// The process ids of every running Roblox Player.
fn player_pids() -> Result<Vec<u32>> {
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
        if is_live_player(&name, entry.cntThreads) {
            pids.push(entry.th32ProcessID);
        }
        // SAFETY: as for Process32FirstW above.
        more = unsafe { Process32NextW(snapshot.0, &mut entry) } == TRUE;
    }

    Ok(pids)
}

/// A process list entry is a running Player: the Player's executable, with
/// threads. A process with none has exited and only waits for its last
/// handle to close, so there is nothing to close.
fn is_live_player(exe: &str, threads: u32) -> bool {
    exe.eq_ignore_ascii_case(PLAYER_EXE) && threads > 0
}

/// One opened process, as far as closing it goes. A trait so the decisions in
/// [`confirm_players`] and [`terminate_players`] can be tested on fakes.
trait Process {
    /// Whether it has exited already, without waiting.
    fn has_exited(&self) -> bool;
    /// The full path of the program it runs.
    fn image(&self) -> std::io::Result<String>;
    fn terminate(&self) -> std::io::Result<()>;
    /// Waits up to 10 seconds; whether it exited.
    fn wait_for_exit(&self) -> bool;
}

/// How long a terminated Player gets to exit.
const EXIT_WAIT_MS: u32 = 10_000;

impl Process for Handle {
    fn has_exited(&self) -> bool {
        // SAFETY: the handle is open, with PROCESS_SYNCHRONIZE, until its
        // guard closes it; a zero timeout only polls.
        unsafe { WaitForSingleObject(self.0, 0) == WAIT_OBJECT_0 }
    }

    fn image(&self) -> std::io::Result<String> {
        image_path(self)
    }

    fn terminate(&self) -> std::io::Result<()> {
        // SAFETY: the handle is open, with PROCESS_TERMINATE, until its guard
        // closes it.
        if unsafe { TerminateProcess(self.0, 1) } == TRUE {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }

    fn wait_for_exit(&self) -> bool {
        // SAFETY: as in `has_exited`, with a bounded wait.
        unsafe { WaitForSingleObject(self.0, EXIT_WAIT_MS) == WAIT_OBJECT_0 }
    }
}

/// Opens a process with the rights closing it takes.
fn open_process(pid: u32) -> std::io::Result<Handle> {
    // SAFETY: OpenProcess takes plain values and returns a handle or null; it
    // reads no memory of ours.
    let handle = unsafe {
        OpenProcess(
            PROCESS_TERMINATE | PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
            0,
            pid,
        )
    };
    if handle.is_null() {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(Handle(handle))
    }
}

/// Every running Player, opened and confirmed, not yet closed. Holding the
/// handles also keeps each process id from being given to another program
/// until they are closed.
struct Players(Vec<(u32, Handle)>);

impl OpenPlayers for Players {
    fn close(self: Box<Self>) -> Result<bool> {
        terminate_players(&self.0)
    }
}

/// Keeps the opened processes that are still the Player, and refuses the
/// whole close, with nothing terminated, when one of them cannot be opened or
/// identified.
fn confirm_players<P: Process>(opened: Vec<(u32, std::io::Result<P>)>) -> Result<Vec<(u32, P)>> {
    let mut players = Vec::new();
    for (pid, process) in opened {
        let process = match process {
            Ok(process) => process,
            Err(error) => {
                not_opened(pid, error)?;
                continue;
            }
        };
        if process.has_exited() {
            continue;
        }
        // The id was the Player's when the processes were listed. If the
        // Player exited since, Windows may have handed the id to another
        // process, which must not be terminated in its place.
        match identify(pid, process.image())? {
            Identity::Player => players.push((pid, process)),
            Identity::Other(image) => println!(
                "{}",
                format!(
                    "Process {pid} is no longer the Roblox Player (it is now {image}); \
                     left running."
                )
                .dimmed()
            ),
        }
    }
    Ok(players)
}

/// Terminates every confirmed Player, then waits for each to exit. `false`
/// when every one had exited on its own.
///
/// Terminated rather than asked to close: a Player that exits normally writes
/// its cookie store first, and that write could land after the new session.
fn terminate_players<P: Process>(players: &[(u32, P)]) -> Result<bool> {
    let mut terminated = false;
    for (pid, process) in players {
        match process.terminate() {
            Ok(()) => terminated = true,
            // TerminateProcess refuses a process that is already exiting.
            Err(_) if process.has_exited() => {}
            Err(error) => bail!(
                "Could not close the Roblox Player (process {pid}): {error}. Close it, then retry."
            ),
        }
    }
    for (pid, process) in players {
        if !process.wait_for_exit() {
            bail!(
                "The Roblox Player (process {pid}) was told to close but had not exited \
                 10 seconds later. Close it, then retry."
            );
        }
    }
    Ok(terminated)
}

/// Why OpenProcess failed on a Player process, from the error it left. `Ok`
/// when the Player is gone.
///
/// Only a process id that no longer names a process means the Player is gone:
/// OpenProcess answers that with ERROR_INVALID_PARAMETER. Any other failure,
/// such as a Player running as administrator refusing an ordinary user, means
/// it is still running, and the switch must not go on as if it were closed.
fn not_opened(pid: u32, error: std::io::Error) -> Result<()> {
    if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
        return Ok(());
    }
    bail!(
        "Could not close the Roblox Player (process {pid}): Windows refused to open it \
         ({error}). Close it yourself, then retry."
    )
}

/// Whose process an opened handle is, from its image path.
#[derive(Debug, PartialEq, Eq)]
enum Identity {
    Player,
    /// Another program, which took the id after the Player exited.
    Other(String),
}

/// Decides from the image path of an opened process whether it is still
/// the Player. A path that cannot be read proves nothing either way, so the
/// switch stops rather than terminate an unknown process or go on with a
/// Player that may still be open.
fn identify(pid: u32, image: std::io::Result<String>) -> Result<Identity> {
    let image = image.map_err(|error| {
        anyhow!(
            "Could not close the Roblox Player (process {pid}): Windows would not say \
             which program it is ({error}). Close it yourself, then retry."
        )
    })?;
    let name = Path::new(&image)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    if name.eq_ignore_ascii_case(PLAYER_EXE) {
        Ok(Identity::Player)
    } else {
        Ok(Identity::Other(image))
    }
}

/// The full path of the program an opened process runs.
fn image_path(process: &Handle) -> std::io::Result<String> {
    read_growing(|buffer| {
        let mut len = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
        // SAFETY: `process` is open with PROCESS_QUERY_LIMITED_INFORMATION
        // until its guard closes it, `buffer` holds at least `len` writable
        // u16s, and `len` is a valid place for the count Windows writes back.
        let ok = unsafe {
            QueryFullProcessImageNameW(process.0, PROCESS_NAME_WIN32, buffer.as_mut_ptr(), &mut len)
        };
        if ok == TRUE {
            Ok(len as usize)
        } else {
            Err(std::io::Error::last_os_error())
        }
    })
}

/// The longest path Windows has: 32,767 characters and the NUL.
const MAX_PATH_CHARS: usize = 32_768;

/// Runs `query` with a buffer that doubles each time it answers
/// ERROR_INSUFFICIENT_BUFFER, up to the longest path Windows allows. `query`
/// returns how many characters it wrote, without the NUL.
fn read_growing(
    mut query: impl FnMut(&mut [u16]) -> std::io::Result<usize>,
) -> std::io::Result<String> {
    let mut buffer = vec![0u16; 260];
    loop {
        match query(&mut buffer) {
            Ok(len) => {
                let written = buffer.get(..len).unwrap_or(&buffer);
                return Ok(String::from_utf16_lossy(written));
            }
            Err(error)
                if error.raw_os_error() == Some(ERROR_INSUFFICIENT_BUFFER as i32)
                    && buffer.len() < MAX_PATH_CHARS =>
            {
                let longer = (buffer.len() * 2).min(MAX_PATH_CHARS);
                buffer.resize(longer, 0);
            }
            Err(error) => return Err(error),
        }
    }
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

    // A stand-in for DPAPI: what it seals, only it unseals.
    fn fake_seal(data: &[u8]) -> Result<Vec<u8>> {
        Ok([b"sealed:".as_slice(), data].concat())
    }

    fn fake_unseal(data: &[u8]) -> Result<Vec<u8>> {
        data.strip_prefix(b"sealed:")
            .map(<[u8]>::to_vec)
            .context("not sealed by the fake")
    }

    fn player_closed() -> Result<bool> {
        Ok(false)
    }

    fn player_open() -> Result<bool> {
        Ok(true)
    }

    /// A store shaped like the Player's: the session among cookies that must
    /// survive the swap untouched.
    const COOKIES: &str = "#HttpOnly_.roblox.com\tTRUE\t/\tTRUE\t0\t.RBXIDCHECK\tid-check; \
        #HttpOnly_.roblox.com\tTRUE\t/\tTRUE\t1828783200\t.ROBLOSECURITY\t_|WARNING:-old-session; \
        #HttpOnly_.roblox.com\tTRUE\t/\tTRUE\t1828783200\trbxas\tabcdef";

    fn new_session() -> Session {
        Session::parse("_|WARNING:-new-session").unwrap()
    }

    /// The store file around `cookies`, sealed with `seal`, with `extra`
    /// spliced in after the two known keys.
    fn store_json(cookies: &str, seal: fn(&[u8]) -> Result<Vec<u8>>, extra: &str) -> String {
        let data = BASE64.encode(seal(cookies.as_bytes()).unwrap());
        format!(r#"{{"CookiesVersion":"1","CookiesData":"{data}"{extra}}}"#)
    }

    /// A store in a temp directory, with the fake cipher.
    fn fake_store(
        contents: &str,
        player_open: fn() -> Result<bool>,
    ) -> (tempfile::TempDir, StoreAccess) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("RobloxCookies.dat");
        fs::write(&path, contents).unwrap();
        let access = StoreAccess {
            path,
            seal: fake_seal,
            unseal: fake_unseal,
            player_open,
        };
        (dir, access)
    }

    /// The names of the files in `dir`, sorted.
    fn files_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_switch_keeps_every_key_and_cookie_it_does_not_own() {
        let before = store_json(COOKIES, fake_seal, r#","LaterKey":{"kept":[1,2]}"#);
        let (_dir, access) = fake_store(&before, player_closed);
        let path = access.path.clone();

        OpenStore::open(access)
            .unwrap()
            .write(&new_session())
            .unwrap();

        let after = fs::read_to_string(&path).unwrap();
        assert!(
            after.starts_with(r#"{"CookiesVersion":"1","CookiesData":""#),
            "the known keys come first, in the Player's order: {after}"
        );
        let after: serde_json::Value = serde_json::from_str(&after).unwrap();
        assert_eq!(after["LaterKey"], serde_json::json!({ "kept": [1, 2] }));

        let data = BASE64
            .decode(after["CookiesData"].as_str().unwrap())
            .unwrap();
        let cookies = String::from_utf8(fake_unseal(&data).unwrap()).unwrap();
        assert_eq!(
            cookies,
            COOKIES.replace("_|WARNING:-old-session", "_|WARNING:-new-session")
        );
    }

    #[test]
    fn the_previous_store_is_kept_beside_it_and_no_temp_file_is_left() {
        let before = store_json(COOKIES, fake_seal, "");
        let (dir, access) = fake_store(&before, player_closed);

        let store = OpenStore::open(access).unwrap();
        store.back_up().unwrap();
        store.write(&new_session()).unwrap();

        assert_eq!(
            files_in(dir.path()),
            ["RobloxCookies.dat", "RobloxCookies.dat.rbx-account.bak"]
        );
        let backup = fs::read_to_string(dir.path().join("RobloxCookies.dat.rbx-account.bak"));
        assert_eq!(backup.unwrap(), before);
    }

    #[test]
    fn a_backup_that_cannot_be_written_is_an_error_and_changes_nothing() {
        let before = store_json(COOKIES, fake_seal, "");
        let (dir, access) = fake_store(&before, player_closed);
        let path = access.path.clone();
        // A directory where the backup goes: the rename cannot replace it.
        fs::create_dir(dir.path().join("RobloxCookies.dat.rbx-account.bak")).unwrap();

        let error = OpenStore::open(access).unwrap().back_up().unwrap_err();

        assert!(error.to_string().contains("backup"), "{error}");
        assert_eq!(fs::read_to_string(path).unwrap(), before);
    }

    #[test]
    fn a_store_that_cannot_take_a_session_is_refused_when_opened() {
        let not_base64 = r#"{"CookiesVersion":"1","CookiesData":"%%%"}"#.to_string();
        let not_sealed = store_json(COOKIES, |data| Ok(data.to_vec()), "");
        let signed_out = store_json(
            "x.com\tTRUE\t/\tFALSE\t0\tGuestData\tUserID=-1",
            fake_seal,
            "",
        );
        let cases = [
            ("not JSON", "{not json".to_string(), "expected format"),
            (
                "no CookiesData",
                r#"{"CookiesVersion":"1"}"#.to_string(),
                "expected format",
            ),
            ("not base64", not_base64, "expected format"),
            ("not sealed", not_sealed, "not sealed"),
            ("signed out", signed_out, "signed out"),
        ];

        for (case, contents, expected) in cases {
            let (dir, access) = fake_store(&contents, player_closed);
            let error = match OpenStore::open(access) {
                Ok(_) => panic!("{case}: opened"),
                Err(error) => format!("{error:#}"),
            };
            assert!(error.contains(expected), "{case}: {error}");
            assert_eq!(files_in(dir.path()), ["RobloxCookies.dat"], "{case}");
        }
    }

    #[test]
    fn a_missing_store_names_its_path() {
        let dir = tempfile::tempdir().unwrap();
        let access = StoreAccess {
            path: dir.path().join("RobloxCookies.dat"),
            seal: fake_seal,
            unseal: fake_unseal,
            player_open: player_closed,
        };
        let error = OpenStore::open(access).err().unwrap().to_string();
        assert!(error.contains("RobloxCookies.dat"), "{error}");
        assert!(files_in(dir.path()).is_empty());
    }

    #[test]
    fn a_player_opened_during_the_switch_leaves_the_store_alone() {
        let before = store_json(COOKIES, fake_seal, "");
        let (dir, access) = fake_store(&before, player_open);
        let path = access.path.clone();

        let error = OpenStore::open(access)
            .unwrap()
            .write(&new_session())
            .unwrap_err();

        assert!(error.to_string().contains("opened during"), "{error}");
        assert_eq!(fs::read_to_string(path).unwrap(), before);
        assert_eq!(files_in(dir.path()), ["RobloxCookies.dat"]);
    }

    /// The store is checked before the ticket is fetched and the Player
    /// closed; what is written is what is on disk at the end.
    #[test]
    fn a_store_rewritten_after_the_check_is_the_one_switched() {
        let before = store_json(COOKIES, fake_seal, "");
        let (dir, access) = fake_store(&before, player_closed);
        let path = access.path.clone();
        let store = OpenStore::open(access).unwrap();
        store.back_up().unwrap();

        let rewritten = store_json(COOKIES, fake_seal, r#","Rewritten":true"#);
        fs::write(&path, &rewritten).unwrap();
        store.write(&new_session()).unwrap();

        let after: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(after["Rewritten"], true);
        let backup = fs::read_to_string(dir.path().join("RobloxCookies.dat.rbx-account.bak"));
        assert_eq!(
            backup.unwrap(),
            before,
            "the backup is the store as checked"
        );
    }

    #[test]
    fn a_failed_replace_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        // A directory where the store should be: the rename cannot replace it.
        let path = dir.path().join("RobloxCookies.dat");
        fs::create_dir(&path).unwrap();

        assert!(replace_synced(&path, b"new").is_err());
        let temps: Vec<_> = files_in(dir.path())
            .into_iter()
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(temps.is_empty(), "{temps:?}");
    }

    #[test]
    fn every_write_has_a_temp_file_of_its_own() {
        let path = Path::new(r"C:\dev\scratch\RobloxCookies.dat");
        assert_ne!(temp_beside(path), temp_beside(path));
        assert_eq!(temp_beside(path).parent(), path.parent());
    }

    /// A temp file that exists is somebody else's: it is never written over.
    #[test]
    fn a_temp_file_that_exists_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let temp = dir.path().join("taken.tmp");
        fs::write(&temp, b"another run's").unwrap();

        assert!(write_new_synced(&temp, b"ours").is_err());
        assert_eq!(fs::read(&temp).unwrap(), b"another run's");
    }

    /// The real cipher, on a store this test sealed itself in a temp
    /// directory.
    #[test]
    fn a_dpapi_store_round_trips() {
        let seal: fn(&[u8]) -> Result<Vec<u8>> = |data| dpapi(data, true);
        let (_dir, mut access) = fake_store(&store_json(COOKIES, seal, ""), player_closed);
        access.seal = seal;
        access.unseal = |data| dpapi(data, false);
        let path = access.path.clone();

        OpenStore::open(access)
            .unwrap()
            .write(&new_session())
            .unwrap();

        let after: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        let data = BASE64
            .decode(after["CookiesData"].as_str().unwrap())
            .unwrap();
        let cookies = String::from_utf8(dpapi(&data, false).unwrap()).unwrap();
        assert!(cookies.contains("\t.ROBLOSECURITY\t_|WARNING:-new-session"));
    }

    #[test]
    fn a_player_that_exited_before_the_close_counts_as_gone() {
        let error = std::io::Error::from_raw_os_error(ERROR_INVALID_PARAMETER as i32);
        assert!(not_opened(1234, error).is_ok());
    }

    const PLAYER_IMAGE: &str = r"C:\Roblox\Versions\version-0\RobloxPlayerBeta.exe";

    /// A process that only records what was done to it.
    struct FakeProcess {
        /// `None` when its image cannot be read.
        image: Option<&'static str>,
        exited: std::cell::Cell<bool>,
        /// Whether TerminateProcess refuses it.
        refuses_terminate: bool,
        terminated: std::cell::Cell<bool>,
    }

    impl FakeProcess {
        fn running(image: &'static str) -> Self {
            Self {
                image: Some(image),
                exited: false.into(),
                refuses_terminate: false,
                terminated: false.into(),
            }
        }
    }

    impl Process for FakeProcess {
        fn has_exited(&self) -> bool {
            self.exited.get()
        }
        fn image(&self) -> std::io::Result<String> {
            self.image
                .map(String::from)
                .ok_or_else(|| std::io::Error::from_raw_os_error(5))
        }
        fn terminate(&self) -> std::io::Result<()> {
            if self.refuses_terminate {
                return Err(std::io::Error::from_raw_os_error(5));
            }
            self.terminated.set(true);
            self.exited.set(true);
            Ok(())
        }
        fn wait_for_exit(&self) -> bool {
            self.exited.get()
        }
    }

    fn gone() -> std::io::Result<FakeProcess> {
        Err(std::io::Error::from_raw_os_error(
            ERROR_INVALID_PARAMETER as i32,
        ))
    }

    #[test]
    fn only_processes_still_the_player_are_kept() {
        let exited = FakeProcess::running(PLAYER_IMAGE);
        exited.exited.set(true);
        let opened = vec![
            (1, Ok(FakeProcess::running(PLAYER_IMAGE))),
            (2, gone()),
            (3, Ok(exited)),
            (4, Ok(FakeProcess::running(r"C:\Windows\notepad.exe"))),
        ];
        let kept: Vec<u32> = confirm_players(opened)
            .unwrap()
            .iter()
            .map(|(pid, _)| *pid)
            .collect();
        assert_eq!(kept, [1]);
    }

    #[test]
    fn one_player_that_cannot_be_confirmed_refuses_the_whole_close() {
        let refused = || Err(std::io::Error::from_raw_os_error(5));
        let unreadable = FakeProcess {
            image: None,
            ..FakeProcess::running(PLAYER_IMAGE)
        };
        for second in [refused(), Ok(unreadable)] {
            let opened = vec![(1, Ok(FakeProcess::running(PLAYER_IMAGE))), (2, second)];
            let error = confirm_players(opened).err().unwrap().to_string();
            assert!(error.contains("process 2"), "{error}");
        }
    }

    #[test]
    fn every_confirmed_player_is_terminated() {
        let players = [
            (1, FakeProcess::running(PLAYER_IMAGE)),
            (2, FakeProcess::running(PLAYER_IMAGE)),
        ];
        assert!(terminate_players(&players).unwrap());
        assert!(players.iter().all(|(_, p)| p.terminated.get()));
    }

    /// TerminateProcess refuses a process that is already exiting; that
    /// one is gone, not a failure.
    #[test]
    fn a_player_that_exits_while_being_closed_is_gone() {
        let exiting = FakeProcess {
            refuses_terminate: true,
            ..FakeProcess::running(PLAYER_IMAGE)
        };
        exiting.exited.set(true);
        assert!(!terminate_players(&[(1, exiting)]).unwrap());
    }

    #[test]
    fn a_player_that_refuses_to_close_and_keeps_running_is_an_error() {
        let stubborn = FakeProcess {
            refuses_terminate: true,
            ..FakeProcess::running(PLAYER_IMAGE)
        };
        let error = terminate_players(&[(1, stubborn)]).unwrap_err().to_string();
        assert!(error.contains("process 1"), "{error}");
    }

    #[test]
    fn a_player_process_with_no_threads_is_not_running() {
        assert!(is_live_player("RobloxPlayerBeta.exe", 12));
        assert!(is_live_player("robloxplayerbeta.exe", 1));
        assert!(!is_live_player("RobloxPlayerBeta.exe", 0));
        assert!(!is_live_player("notepad.exe", 12));
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
    fn the_player_is_recognised_by_its_image_whatever_the_case() {
        let image = r"C:\Program Files\Roblox\Versions\version-0\RobloxPlayerBeta.exe";
        assert_eq!(identify(1234, Ok(image.into())).unwrap(), Identity::Player);
        let lower = r"c:\roblox\robloxplayerbeta.EXE";
        assert_eq!(identify(1234, Ok(lower.into())).unwrap(), Identity::Player);
    }

    /// An id the Player held, handed to another program after it exited.
    #[test]
    fn a_reused_process_id_is_another_program() {
        let image = r"C:\Windows\System32\notepad.exe";
        assert_eq!(
            identify(1234, Ok(image.into())).unwrap(),
            Identity::Other(image.into())
        );
        let lookalike = r"C:\x\RobloxPlayerBeta.exe.old";
        assert!(matches!(
            identify(1234, Ok(lookalike.into())).unwrap(),
            Identity::Other(_)
        ));
    }

    /// A query that answers ERROR_INSUFFICIENT_BUFFER until the buffer holds
    /// `path`, the way QueryFullProcessImageNameW does.
    fn query_for(path: &str) -> impl FnMut(&mut [u16]) -> std::io::Result<usize> + '_ {
        move |buffer| {
            let wide: Vec<u16> = path.encode_utf16().collect();
            if buffer.len() <= wide.len() {
                return Err(std::io::Error::from_raw_os_error(
                    ERROR_INSUFFICIENT_BUFFER as i32,
                ));
            }
            buffer[..wide.len()].copy_from_slice(&wide);
            Ok(wide.len())
        }
    }

    #[test]
    fn a_long_image_path_is_read_whole() {
        let long = format!(r"C:\{}\RobloxPlayerBeta.exe", "d".repeat(2000));
        assert_eq!(read_growing(query_for(&long)).unwrap(), long);
    }

    #[test]
    fn a_path_longer_than_windows_allows_is_an_error() {
        let endless = "d".repeat(MAX_PATH_CHARS);
        assert!(read_growing(query_for(&endless)).is_err());
    }

    #[test]
    fn a_process_whose_image_cannot_be_read_stops_the_switch() {
        const ERROR_ACCESS_DENIED: i32 = 5;
        let error = std::io::Error::from_raw_os_error(ERROR_ACCESS_DENIED);
        let error = identify(1234, Err(error)).unwrap_err().to_string();
        assert!(error.contains("process 1234"), "{error}");
    }

    #[test]
    fn a_malformed_registry_entry_is_an_error() {
        assert!(WindowsCredentialStore::parse_accounts_from_registry("{not json").is_err());
    }
}
