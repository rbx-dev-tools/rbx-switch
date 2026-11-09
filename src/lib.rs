//! Switch between Roblox Studio accounts. Cross-platform credential store
//! (Windows registry + Credential Manager today; macOS stub). With
//! `--player`, the same accounts switch the Roblox Player too.

mod config;
mod credential;
mod player;

#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "macos")]
mod macos;

use anyhow::{anyhow, bail, Result};
use clap::{Args, Subcommand};
use colored::Colorize;
use dialoguer::{theme::ColorfulTheme, Select};

use crate::config::{Config, Source};
use crate::credential::{Account, CredentialStore};

/// Cross-cutting flags, kept in one `Args` group so they can be placed either
/// before or after the subcommand: `rbx-account --no-color list` and
/// `rbx-account list --no-color` both work.
///
/// An earlier version accepted seven more flags, about Open Cloud auth and
/// place resolution, and never read any of them. The two below are the ones a
/// Studio account switcher genuinely has, and both are wired to behaviour
/// rather than accepted and dropped.
#[derive(Args, Debug, Clone, Default)]
pub struct GlobalFlags {
    /// Never colour the output.
    ///
    /// For logs, pipes and terminals that render ANSI escapes literally. The
    /// `NO_COLOR` environment variable is honoured too, without this flag.
    #[arg(long, global = true)]
    pub no_color: bool,

    /// Fail instead of opening the interactive picker.
    ///
    /// The picker is what `rbx-account` does when given no account name, and it
    /// blocks forever on a machine with no terminal. In a script, pass this so
    /// a missing name is an error you can see rather than a hung job.
    #[arg(long, global = true)]
    pub non_interactive: bool,

    /// Print machine-readable JSON instead of the human listing.
    ///
    /// For `list` and `current`. The human output is a layout, and a layout is
    /// free to change; this is the shape a script may depend on. Colour is
    /// suppressed with it, since a caller parsing JSON never wants escapes.
    #[arg(long, global = true)]
    pub json: bool,
}

/// Which app a switch applies to. Studio when neither flag is given, so
/// every command written before `--player` existed means what it meant.
///
/// Flags rather than a `studio` / `player` subcommand level: the account is
/// what the command is about, and the Player has no account list of its own.
/// Any account with a Studio session can be switched to in either app.
#[derive(Args, Debug, Clone, Copy, Default)]
pub struct Targets {
    /// Switch Studio. The default; only needed alongside `--player`, to
    /// switch both.
    #[arg(long, global = true)]
    pub studio: bool,

    /// Switch the Roblox Player instead of Studio.
    ///
    /// Nothing is opened: the closed Player starts signed in as the account
    /// next time, like a Studio switch. It gets a session of its own, from a
    /// sign-in ticket requested with the account's Studio session. Refused
    /// while a Player is open, unless `--force`. Windows only.
    #[arg(long, global = true)]
    pub player: bool,

    /// With `--player`, open the Player signed in as the account now.
    ///
    /// The only way to switch an open Player: a new one starts, and the open
    /// one exits a few seconds later, game in progress included.
    #[arg(long, global = true)]
    pub force: bool,
}

impl Targets {
    fn studio(self) -> bool {
        self.studio || !self.player
    }
}

#[derive(Args, Debug)]
pub struct SwitchCli {
    #[command(subcommand)]
    pub command: Option<SwitchCommands>,

    /// Switch directly by alias or username (shorthand for `rbx-account use <name>`).
    pub name: Option<String>,

    #[command(flatten)]
    pub targets: Targets,
}

#[derive(Subcommand, Debug)]
pub enum SwitchCommands {
    /// List all signed-in accounts.
    List,

    /// Show the currently active account.
    Current,

    /// Switch to a specific account by alias, username, or user id.
    Use {
        /// Alias, username, or user id.
        name: String,
    },
}

pub fn run(cli: SwitchCli, global: &GlobalFlags) -> Result<()> {
    if global.no_color || global.json {
        colored::control::set_override(false);
    }

    let targets = cli.targets;
    if targets.force && !targets.player {
        bail!("--force only applies to --player: a Studio switch closes nothing.");
    }
    if targets.player
        && matches!(
            cli.command,
            Some(SwitchCommands::List | SwitchCommands::Current)
        )
    {
        // Refused rather than ignored: `current --player` printing Studio's
        // account would be a confident wrong answer.
        bail!(
            "`list` and `current` read Studio only: the Player keeps no account this \
             tool can read back. --player applies to switching."
        );
    }

    let store = get_store()?;
    let config = Config::load()?;

    match cli.command {
        Some(SwitchCommands::List) if global.json => json_list(&*store),
        Some(SwitchCommands::Current) if global.json => json_current(&*store),
        Some(SwitchCommands::List) => cmd_list(&*store, &config),
        Some(SwitchCommands::Current) => cmd_current(&*store),
        Some(SwitchCommands::Use { name }) => cmd_switch(&*store, &config, &name, targets),
        None => {
            if let Some(name) = cli.name {
                cmd_switch(&*store, &config, &name, targets)
            } else {
                cmd_interactive(&*store, global, targets)
            }
        }
    }
}

/// Pick the credential store for this platform.
///
/// Fallible rather than infallible so that a Linux user gets the sentence
/// below instead of a backtrace: an unsupported platform is a thing the user
/// can act on, which makes it an error, not a bug.
fn get_store() -> Result<Box<dyn CredentialStore>> {
    #[cfg(target_os = "windows")]
    {
        Ok(Box::new(windows::WindowsCredentialStore::new()))
    }
    #[cfg(target_os = "macos")]
    {
        Ok(Box::new(macos::MacCredentialStore::new()))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        anyhow::bail!(
            "rbx-account is not supported on this platform.\n\
             Studio stores its signed-in accounts in the Windows registry or the \
             macOS keychain, and neither exists here."
        )
    }
}

/// `list` as one JSON document.
///
/// A document rather than a line-per-account stream: the account list is
/// small, fixed and read whole, so there is nothing to stream and a caller
/// gets to parse it in one call.
fn json_list(store: &dyn CredentialStore) -> Result<()> {
    let accounts = store.list_accounts()?;
    let current = store.current_user_id()?;

    let rows: Vec<_> = accounts
        .iter()
        .map(|account| {
            serde_json::json!({
                "user_id": account.user_id,
                "username": account.username,
                "active": current.as_ref() == Some(&account.user_id),
            })
        })
        .collect();

    println!("{}", serde_json::json!({ "accounts": rows }));
    Ok(())
}

/// `current` as one JSON document. `account` is `null` when Studio has no
/// active account, which a caller has to handle either way.
fn json_current(store: &dyn CredentialStore) -> Result<()> {
    let accounts = store.list_accounts()?;
    let current = store.current_user_id()?;

    let account = current.map(|id| {
        let username = accounts
            .iter()
            .find(|a| a.user_id == id)
            .map(|a| a.username.clone());
        serde_json::json!({ "user_id": id, "username": username })
    });

    println!("{}", serde_json::json!({ "account": account }));
    Ok(())
}

fn cmd_list(store: &dyn CredentialStore, config: &Config) -> Result<()> {
    let accounts = store.list_accounts()?;
    let current = store.current_user_id()?;

    if accounts.is_empty() {
        println!(
            "{}",
            "No accounts found. Log into Roblox Studio first.".yellow()
        );
        return Ok(());
    }

    for account in &accounts {
        let is_active = current.as_ref() == Some(&account.user_id);
        let marker = if is_active {
            " *".green().to_string()
        } else {
            String::new()
        };
        println!(
            "  {} ({}){marker}",
            account.username.cyan(),
            account.user_id.dimmed(),
        );
    }

    // Naming the files that are in effect answers "why does `dev` mean that
    // here" without making the reader guess which of the two won, and it is
    // the only place a project file the reader did not know about shows up.
    let sources = [
        config.project_path().map(|p| ("project", p)),
        config.global_path().map(|p| ("personal", p)),
    ];
    if sources.iter().any(Option::is_some) {
        println!();
        for (label, path) in sources.into_iter().flatten() {
            println!(
                "  {}",
                format!("{label} aliases: {}", path.display()).dimmed()
            );
        }
    }

    Ok(())
}

fn cmd_current(store: &dyn CredentialStore) -> Result<()> {
    let accounts = store.list_accounts()?;
    let current = store.current_user_id()?;

    match current {
        Some(id) => {
            let name = accounts
                .iter()
                .find(|a| a.user_id == id)
                .map(|a| a.username.as_str())
                .unwrap_or("unknown");
            println!("{} ({})", name.cyan(), id.dimmed());
        }
        None => println!("{}", "No active account.".yellow()),
    }

    Ok(())
}

fn cmd_switch(
    store: &dyn CredentialStore,
    config: &Config,
    name: &str,
    targets: Targets,
) -> Result<()> {
    let accounts = store.list_accounts()?;
    let resolved = resolve_account(&accounts, config, name)?;

    let account = accounts
        .iter()
        .find(|a| a.user_id == resolved.user_id)
        .ok_or_else(|| anyhow!("Account not found: {name}"))?;

    switch(store, account, targets)?;

    // Which file named this account, and the reminder that the switch outlives
    // the directory it was made in. Studio and the Player each have one
    // signed-in account for the whole machine, so a project-local alias can
    // leave the next checkout signed in as somebody it never mentions.
    if let Some(source) = resolved.via {
        println!(
            "  {} {} {}",
            "alias from".dimmed(),
            source.describe().dimmed(),
            format!("- {} until you switch again", stays_on(targets)).dimmed(),
        );
    }

    Ok(())
}

fn stays_on(targets: Targets) -> &'static str {
    match (targets.studio(), targets.player) {
        (true, true) => "Studio and the Player stay on this account",
        (false, true) => "the Player stays on this account",
        _ => "Studio stays on this account",
    }
}

/// How a Player switch lands.
enum PlayerSwitch {
    /// The Player is closed: its cookie store gets this session, and it opens
    /// signed in with it next time. Nothing is launched.
    Store(player::Session),
    /// `--force`: this `roblox-player:` URI starts the Player signed in.
    Launch(String),
}

/// Applies one switch to every target.
///
/// Everything that can fail without changing anything runs first: the
/// Player check, the session lookup, the ticket and its redeem. Then the
/// Player's switch, the step most likely to fail, and Studio's last, so a
/// refusal leaves both apps as they were rather than Studio switched and the
/// Player not.
fn switch(store: &dyn CredentialStore, account: &Account, targets: Targets) -> Result<()> {
    let player = if targets.player {
        Some(player_prepare(store, account, targets.force)?)
    } else {
        None
    };

    match player {
        Some(PlayerSwitch::Store(session)) => {
            store.write_player_session(&session)?;
            println!(
                "{} {} ({}) {}",
                "Switched the Player to".green(),
                account.username.cyan(),
                account.user_id.dimmed(),
                "- it opens signed in next time".dimmed(),
            );
        }
        Some(PlayerSwitch::Launch(uri)) => {
            store.open_player(&uri)?;
            println!(
                "{} {} ({}) {}",
                "Switched the Player to".green(),
                account.username.cyan(),
                account.user_id.dimmed(),
                "- it opens signed in within a few seconds".dimmed(),
            );
        }
        None => {}
    }

    if targets.studio() {
        store.set_user_id(&account.user_id)?;
        let label = if targets.player {
            "Switched Studio to"
        } else {
            "Switched to"
        };
        println!(
            "{} {} ({})",
            label.green(),
            account.username.cyan(),
            account.user_id.dimmed(),
        );
    }

    Ok(())
}

/// Everything a Player switch needs, fetched before anything changes.
fn player_prepare(
    store: &dyn CredentialStore,
    account: &Account,
    force: bool,
) -> Result<PlayerSwitch> {
    store.ensure_player_installed()?;

    // An open Player writes its cookie store back when it exits, over
    // anything written meanwhile, so the store can only be switched closed.
    // A ticket launch does reach it, by starting a new Player: the open one
    // exits 5 to 9 seconds later (measured twice on 2026-11-08), disconnecting
    // any game in progress. Checked before the ticket is requested, so a
    // refusal spends nothing.
    if !force && store.player_running()? {
        bail!(
            "The Roblox Player is open, and it rewrites its sign-in when it exits. \
             Close it first, or pass --force to restart it signed in, game in \
             progress included."
        );
    }

    let who = &account.username;
    let session = store.session(&account.user_id)?.ok_or_else(|| {
        anyhow!(
            "Studio lists {who} but holds no session for it. \
             Sign in to Studio as {who} once, then retry."
        )
    })?;

    let transport = player::HttpTransport::new();
    let ticket = player::request_ticket(&transport, &session, who)?;
    if force {
        return Ok(PlayerSwitch::Launch(player::launch_uri(
            &ticket,
            player::now_ms(),
        )));
    }

    let player_session = player::redeem_ticket(&transport, &ticket)?;
    let owner = transport.user_id(&player_session)?;
    if owner != account.user_id {
        bail!(
            "Roblox issued a session for user {owner}, not {who} ({}). Nothing was changed.",
            account.user_id
        );
    }
    Ok(PlayerSwitch::Store(player_session))
}

fn cmd_interactive(
    store: &dyn CredentialStore,
    global: &GlobalFlags,
    targets: Targets,
) -> Result<()> {
    let accounts = store.list_accounts()?;
    // The `*` marks Studio's account. For a Player-only switch there is no
    // known current account to mark, and marking Studio's would suggest the
    // Player is on it.
    let current = if targets.studio() {
        store.current_user_id()?
    } else {
        None
    };

    if accounts.is_empty() {
        println!(
            "{}",
            "No accounts found. Log into Roblox Studio first.".yellow()
        );
        return Ok(());
    }

    if global.non_interactive {
        bail!(
            "No account named, and --non-interactive forbids the picker. \
             Pass an alias, username or user id, or run `rbx-account list` to see them."
        );
    }

    let items: Vec<String> = accounts
        .iter()
        .map(|a| {
            let marker = if current.as_ref() == Some(&a.user_id) {
                " *"
            } else {
                ""
            };
            format!("{} ({}){marker}", a.username, a.user_id)
        })
        .collect();

    let default = current
        .as_ref()
        .and_then(|id| accounts.iter().position(|a| &a.user_id == id))
        .unwrap_or(0);

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select account")
        .items(&items)
        .default(default)
        .interact_opt()?;

    if let Some(idx) = selection {
        switch(store, &accounts[idx], targets)?;
    }

    Ok(())
}

/// How a name was turned into a user id, so the caller can say it out loud.
///
/// Only the alias case carries anything: a user id or a username is its own
/// explanation, while an alias is a project-local name for machine-global
/// state and the reader deserves to know which file chose it.
#[derive(Debug)]
struct Resolved {
    user_id: String,
    via: Option<Source>,
}

fn resolve_account(accounts: &[Account], config: &Config, name: &str) -> Result<Resolved> {
    // 1. Direct user ID
    if accounts.iter().any(|a| a.user_id == name) {
        return Ok(Resolved {
            user_id: name.to_string(),
            via: None,
        });
    }

    // 2. Alias from config
    if let Some((id, source)) = config.resolve_alias(name) {
        if accounts.iter().any(|a| a.user_id == id) {
            return Ok(Resolved {
                user_id: id,
                via: Some(source),
            });
        }
    }

    // 3. Username (case-insensitive)
    let lower = name.to_lowercase();
    if let Some(account) = accounts.iter().find(|a| a.username.to_lowercase() == lower) {
        return Ok(Resolved {
            user_id: account.user_id.clone(),
            via: None,
        });
    }

    Err(anyhow!(
        "No account found matching '{name}'. Use `rbx-account list` to see available accounts."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(user_id: &str, username: &str) -> Account {
        Account {
            user_id: user_id.to_string(),
            username: username.to_string(),
            profile_pic_url: None,
        }
    }

    fn accounts() -> Vec<Account> {
        vec![
            account("1122334455", "MainAccount"),
            account("9876543210", "DevAccount"),
        ]
    }

    #[test]
    fn a_user_id_resolves_to_itself() {
        let config = Config::default();
        assert_eq!(
            resolve_account(&accounts(), &config, "9876543210")
                .unwrap()
                .user_id,
            "9876543210"
        );
    }

    #[test]
    fn a_username_resolves_regardless_of_case() {
        let config = Config::default();
        assert_eq!(
            resolve_account(&accounts(), &config, "mainaccount")
                .unwrap()
                .user_id,
            "1122334455"
        );
    }

    #[test]
    fn an_alias_resolves_through_the_config() {
        let config = Config::with_alias("dev", 9_876_543_210, Source::Project);
        let resolved = resolve_account(&accounts(), &config, "dev").unwrap();
        assert_eq!(resolved.user_id, "9876543210");
        assert_eq!(
            resolved.via,
            Some(Source::Project),
            "the switch line has to be able to say which file named this account"
        );
    }

    /// An alias pointing at an account that is no longer signed in must fall
    /// through to the username pass rather than resolve to a dead id: the
    /// caller would then be told "Account not found" for a name that does
    /// exist.
    #[test]
    fn an_alias_for_a_signed_out_account_falls_through() {
        let config = Config::with_alias("gone", 1, Source::Global);
        let error = resolve_account(&accounts(), &config, "gone").unwrap_err();
        assert!(
            error.to_string().contains("No account found matching"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn an_unknown_name_names_the_command_that_lists_accounts() {
        let config = Config::default();
        let error = resolve_account(&accounts(), &config, "nobody").unwrap_err();
        assert!(
            error.to_string().contains("rbx-account list"),
            "unexpected error: {error}"
        );
    }
}
