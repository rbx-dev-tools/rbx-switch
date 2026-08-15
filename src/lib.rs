//! Switch between Roblox Studio accounts. Cross-platform credential store
//! (Windows registry + Credential Manager today; macOS stub).

mod config;
mod credential;

#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "macos")]
mod macos;

use anyhow::{anyhow, bail, Result};
use clap::{Args, Subcommand};
use colored::Colorize;
use dialoguer::{theme::ColorfulTheme, Select};

use crate::config::Config;
use crate::credential::{Account, CredentialStore};

/// Cross-cutting flags, kept in one `Args` group so they can be placed either
/// before or after the subcommand: `rbx-switch --no-color list` and
/// `rbx-switch list --no-color` both work.
///
/// This is what is left of the `rbx_core::GlobalFlags` this crate used to
/// borrow from the `rbx-cli` workspace. That struct carried seven flags about
/// Open Cloud auth and `rbxplace.toml` resolution, none of which this tool
/// ever read — it took the reference and ignored it. Standing alone, the two
/// flags below are the ones a Studio account switcher genuinely has, and both
/// are wired to behaviour rather than accepted and dropped.
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
    /// The picker is what `rbx-switch` does when given no account name, and it
    /// blocks forever on a machine with no terminal. In a script, pass this so
    /// a missing name is an error you can see rather than a hung job.
    #[arg(long, global = true)]
    pub non_interactive: bool,
}

#[derive(Args, Debug)]
pub struct SwitchCli {
    #[command(subcommand)]
    pub command: Option<SwitchCommands>,

    /// Switch directly by alias or username (shorthand for `rbx-switch use <name>`).
    pub name: Option<String>,
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
    if global.no_color {
        colored::control::set_override(false);
    }

    let store = get_store()?;
    let config = Config::load()?;

    match cli.command {
        Some(SwitchCommands::List) => cmd_list(&*store),
        Some(SwitchCommands::Current) => cmd_current(&*store),
        Some(SwitchCommands::Use { name }) => cmd_switch(&*store, &config, &name),
        None => {
            if let Some(name) = cli.name {
                cmd_switch(&*store, &config, &name)
            } else {
                cmd_interactive(&*store, global)
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
            "rbx-switch is not supported on this platform.\n\
             Studio stores its signed-in accounts in the Windows registry or the \
             macOS keychain, and neither exists here."
        )
    }
}

fn cmd_list(store: &dyn CredentialStore) -> Result<()> {
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

fn cmd_switch(store: &dyn CredentialStore, config: &Config, name: &str) -> Result<()> {
    let accounts = store.list_accounts()?;
    let user_id = resolve_account(&accounts, config, name)?;

    let account = accounts
        .iter()
        .find(|a| a.user_id == user_id)
        .ok_or_else(|| anyhow!("Account not found: {name}"))?;

    store.set_user_id(&user_id)?;
    println!(
        "{} {} ({})",
        "Switched to".green(),
        account.username.cyan(),
        account.user_id.dimmed(),
    );

    Ok(())
}

fn cmd_interactive(store: &dyn CredentialStore, global: &GlobalFlags) -> Result<()> {
    let accounts = store.list_accounts()?;
    let current = store.current_user_id()?;

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
             Pass an alias, username or user id, or run `rbx-switch list` to see them."
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
        let account = &accounts[idx];
        store.set_user_id(&account.user_id)?;
        println!(
            "{} {} ({})",
            "Switched to".green(),
            account.username.cyan(),
            account.user_id.dimmed(),
        );
    }

    Ok(())
}

fn resolve_account(accounts: &[Account], config: &Config, name: &str) -> Result<String> {
    // 1. Direct user ID
    if accounts.iter().any(|a| a.user_id == name) {
        return Ok(name.to_string());
    }

    // 2. Alias from config
    if let Some(id) = config.resolve_alias(name) {
        if accounts.iter().any(|a| a.user_id == id) {
            return Ok(id);
        }
    }

    // 3. Username (case-insensitive)
    let lower = name.to_lowercase();
    if let Some(account) = accounts.iter().find(|a| a.username.to_lowercase() == lower) {
        return Ok(account.user_id.clone());
    }

    Err(anyhow!(
        "No account found matching '{name}'. Use `rbx-switch list` to see available accounts."
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
            resolve_account(&accounts(), &config, "9876543210").unwrap(),
            "9876543210"
        );
    }

    #[test]
    fn a_username_resolves_regardless_of_case() {
        let config = Config::default();
        assert_eq!(
            resolve_account(&accounts(), &config, "mainaccount").unwrap(),
            "1122334455"
        );
    }

    #[test]
    fn an_alias_resolves_through_the_config() {
        let mut config = Config::default();
        config.aliases.insert("dev".to_string(), 9_876_543_210);
        assert_eq!(
            resolve_account(&accounts(), &config, "dev").unwrap(),
            "9876543210"
        );
    }

    /// An alias pointing at an account that is no longer signed in must fall
    /// through to the username pass rather than resolve to a dead id: the
    /// caller would then be told "Account not found" for a name that does
    /// exist.
    #[test]
    fn an_alias_for_a_signed_out_account_falls_through() {
        let mut config = Config::default();
        config.aliases.insert("gone".to_string(), 1);
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
            error.to_string().contains("rbx-switch list"),
            "unexpected error: {error}"
        );
    }
}
