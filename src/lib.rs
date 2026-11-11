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

use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Subcommand};
use colored::Colorize;
use dialoguer::{theme::ColorfulTheme, Select};

use crate::config::{Config, Source};
use crate::credential::{Account, CredentialStore, OpenPlayers, PlayerStore};

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
    /// Nothing is opened: the Player starts signed in as the account next
    /// time, like a Studio switch. It gets a session of its own, from a
    /// sign-in ticket requested with the account's Studio session. Refused
    /// while a Player is open, unless `--force`. Windows only.
    #[arg(long, global = true)]
    pub player: bool,

    /// With `--player`, close an open Player first, game in progress
    /// included. It is not reopened.
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

/// Trades an account's Studio session for a Player session of its own. A
/// parameter of [`switch_with`], so the order of a switch can be tested
/// without a network.
type Exchange<'a> = &'a dyn Fn(&player::Session, &Account) -> Result<player::Session>;

fn switch(store: &dyn CredentialStore, account: &Account, targets: Targets) -> Result<()> {
    switch_with(store, account, targets, &exchange_session)
}

/// Applies one switch to every target.
///
/// Everything that can fail without changing anything runs first: the
/// Player check, the session lookup, reading and checking the Player's
/// cookie store, with `--force` opening and identifying every open Player,
/// the backup of the store, then the ticket and its redeem. Only then does anything change: the open
/// Players are closed, the Player's store is written, and Studio is switched
/// last. A refusal at any earlier step leaves both apps as they were, and a
/// store that could not take the session, or a Player that could not be
/// closed, costs no ticket.
fn switch_with(
    store: &dyn CredentialStore,
    account: &Account,
    targets: Targets,
    exchange: Exchange<'_>,
) -> Result<()> {
    if targets.player {
        let prepared = player_prepare(store, account, targets.force, exchange)?;
        if let Some(players) = prepared.players {
            if players.close()? {
                println!("{}", "Closed the Roblox Player".yellow());
            }
        }
        prepared.store.write_session(&prepared.session)?;
        println!(
            "{} {} ({}) {}",
            "Switched the Player to".green(),
            account.username.cyan(),
            account.user_id.dimmed(),
            "- it opens signed in next time".dimmed(),
        );
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

/// What a Player switch has in hand before it changes anything.
struct Prepared {
    /// The Player's store, read and checked.
    store: Box<dyn PlayerStore>,
    /// With `--force`, every open Player, confirmed and not yet closed.
    players: Option<Box<dyn OpenPlayers>>,
    /// The Player's new session.
    session: player::Session,
}

fn player_prepare(
    store: &dyn CredentialStore,
    account: &Account,
    force: bool,
    exchange: Exchange<'_>,
) -> Result<Prepared> {
    store.ensure_player_installed()?;

    // An open Player writes its cookie store back when it exits, over
    // anything written meanwhile, so the store can only be switched closed.
    // Checked before the ticket is requested, so a refusal spends nothing.
    if !force && store.player_running()? {
        bail!(
            "The Roblox Player is open, and it rewrites its sign-in when it exits. \
             Close it first, or pass --force to close it, game in progress included."
        );
    }

    let who = &account.username;
    let session = store.session(&account.user_id)?.ok_or_else(|| {
        anyhow!(
            "Studio lists {who} but holds no session for it. \
             Sign in to Studio as {who} once, then retry."
        )
    })?;

    // Before the ticket: a store that is missing, unreadable or signed out
    // refuses here, with no ticket spent and no Player closed.
    let player_store = store.open_player_store()?;
    // Also before the ticket: a Player that could not be closed refuses here,
    // rather than after a session was minted for nothing.
    let players = if force {
        Some(store.open_players()?)
    } else {
        None
    };
    // The last step before the irreversible ones: a backup that cannot be
    // written stops the switch with nothing changed.
    player_store.back_up()?;
    let player_session = exchange(&session, account)?;

    // A Player started during the exchange would write its store back over
    // the switch, so it is confirmed now and closed with the others; if it
    // cannot be, nothing has been closed yet.
    let mut players = players;
    if let Some(players) = &mut players {
        players.add_newcomers().context(
            "A Player was opened during the switch and cannot be closed. Nothing was \
             closed or written, and the new Player session was discarded",
        )?;
    }
    Ok(Prepared {
        store: player_store,
        players,
        session: player_session,
    })
}

/// The real [`Exchange`]: a ticket from the Studio session, redeemed for a
/// Player session, checked to be the account's.
fn exchange_session(session: &player::Session, account: &Account) -> Result<player::Session> {
    let who = &account.username;
    let transport = player::HttpTransport::new();
    let ticket = player::request_ticket(&transport, session, who)?;
    let player_session = player::redeem_ticket(&transport, &ticket)?;
    let owner = transport.user_id(&player_session)?;
    if owner != account.user_id {
        bail!(
            "Roblox issued a session for user {owner}, not {who} ({}). Nothing was changed.",
            account.user_id
        );
    }
    Ok(player_session)
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
    use std::cell::RefCell;
    use std::rc::Rc;

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

    /// Records every step a switch takes, and fails the one named.
    #[derive(Clone, Default)]
    struct Steps {
        taken: Rc<RefCell<Vec<&'static str>>>,
        failing: Option<&'static str>,
    }

    impl Steps {
        fn take(&self, step: &'static str) -> Result<()> {
            self.taken.borrow_mut().push(step);
            if self.failing == Some(step) {
                bail!("{step} failed");
            }
            Ok(())
        }
    }

    /// A credential store that touches nothing, for the order of a switch.
    struct Fake {
        steps: Steps,
        player_running: bool,
    }

    impl CredentialStore for Fake {
        fn list_accounts(&self) -> Result<Vec<Account>> {
            Ok(accounts())
        }
        fn current_user_id(&self) -> Result<Option<String>> {
            Ok(None)
        }
        fn set_user_id(&self, _: &str) -> Result<()> {
            self.steps.take("set_user_id")
        }
        fn session(&self, _: &str) -> Result<Option<player::Session>> {
            self.steps.take("session")?;
            Ok(player::Session::parse("_|WARNING:-studio-session"))
        }
        fn ensure_player_installed(&self) -> Result<()> {
            self.steps.take("ensure_player_installed")
        }
        fn player_running(&self) -> Result<bool> {
            self.steps.take("player_running")?;
            Ok(self.player_running)
        }
        fn open_players(&self) -> Result<Box<dyn OpenPlayers>> {
            self.steps.take("open_players")?;
            Ok(Box::new(FakePlayers(self.steps.clone())))
        }
        fn open_player_store(&self) -> Result<Box<dyn PlayerStore>> {
            self.steps.take("open_player_store")?;
            Ok(Box::new(FakePlayerStore(self.steps.clone())))
        }
    }

    struct FakePlayers(Steps);

    impl OpenPlayers for FakePlayers {
        fn add_newcomers(&mut self) -> Result<()> {
            self.0.take("add_newcomers")
        }

        fn close(self: Box<Self>) -> Result<bool> {
            self.0.take("close_players")?;
            Ok(true)
        }
    }

    struct FakePlayerStore(Steps);

    impl PlayerStore for FakePlayerStore {
        fn back_up(&self) -> Result<()> {
            self.0.take("back_up")
        }

        fn write_session(self: Box<Self>, _: &player::Session) -> Result<()> {
            self.0.take("write_session")
        }
    }

    /// Switches to the first account with `targets`, the Player open, and
    /// `failing` failing. Returns the outcome and every step taken.
    fn switch_fake(
        targets: Targets,
        failing: Option<&'static str>,
    ) -> (Result<()>, Vec<&'static str>) {
        let steps = Steps {
            failing,
            ..Steps::default()
        };
        let store = Fake {
            steps: steps.clone(),
            player_running: true,
        };
        let exchange = |_: &player::Session, _: &Account| {
            steps.take("exchange")?;
            Ok(player::Session::parse("_|WARNING:-player-session").unwrap())
        };
        let result = switch_with(&store, &accounts()[0], targets, &exchange);
        let taken = steps.taken.borrow().clone();
        (result, taken)
    }

    const BOTH_FORCED: Targets = Targets {
        studio: true,
        player: true,
        force: true,
    };

    #[test]
    fn a_switch_checks_everything_before_it_changes_anything() {
        let (result, steps) = switch_fake(BOTH_FORCED, None);
        result.unwrap();
        assert_eq!(
            steps,
            [
                "ensure_player_installed",
                "session",
                "open_player_store",
                "open_players",
                "back_up",
                "exchange",
                "add_newcomers",
                "close_players",
                "write_session",
                "set_user_id",
            ]
        );
    }

    #[test]
    fn a_backup_that_cannot_be_written_stops_the_switch_before_the_ticket() {
        let (result, steps) = switch_fake(BOTH_FORCED, Some("back_up"));
        assert!(result.is_err());
        assert_eq!(steps.last(), Some(&"back_up"));
        assert!(!steps.contains(&"exchange"), "{steps:?}");
    }

    #[test]
    fn a_player_opened_during_the_exchange_that_cannot_be_closed_closes_nothing() {
        let (result, steps) = switch_fake(BOTH_FORCED, Some("add_newcomers"));
        let error = format!("{:#}", result.unwrap_err());
        assert!(error.contains("discarded"), "{error}");
        assert_eq!(steps.last(), Some(&"add_newcomers"));
        assert!(!steps.contains(&"close_players"), "{steps:?}");
    }

    #[test]
    fn a_player_that_cannot_be_closed_spends_no_ticket() {
        let (result, steps) = switch_fake(BOTH_FORCED, Some("open_players"));
        assert!(result.is_err());
        assert_eq!(steps.last(), Some(&"open_players"));
        assert!(!steps.contains(&"exchange"), "{steps:?}");
    }

    #[test]
    fn a_store_that_cannot_take_the_session_spends_no_ticket_and_closes_nothing() {
        let (result, steps) = switch_fake(BOTH_FORCED, Some("open_player_store"));
        assert!(result.is_err());
        assert_eq!(
            steps,
            ["ensure_player_installed", "session", "open_player_store"]
        );
    }

    #[test]
    fn a_refused_ticket_closes_nothing() {
        let (result, steps) = switch_fake(BOTH_FORCED, Some("exchange"));
        assert!(result.is_err());
        assert_eq!(steps.last(), Some(&"exchange"));
    }

    #[test]
    fn a_store_write_that_fails_leaves_studio_alone() {
        let (result, steps) = switch_fake(BOTH_FORCED, Some("write_session"));
        assert!(result.is_err());
        assert!(!steps.contains(&"set_user_id"), "{steps:?}");
    }

    #[test]
    fn an_open_player_without_force_is_refused_first() {
        let targets = Targets {
            player: true,
            ..Targets::default()
        };
        let (result, steps) = switch_fake(targets, None);
        let error = result.unwrap_err().to_string();
        assert!(error.contains("--force"), "{error}");
        assert_eq!(steps, ["ensure_player_installed", "player_running"]);
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
