//! Alias resolution, layered like `git config`.
//!
//! An alias is a name for an account, and names belong to projects: `dev` on a
//! personal game and `dev` on a client's are rarely the same Roblox account. So
//! a project file wins over a personal one, key by key.
//!
//! Which file answered is not a detail. This tool changes machine-global state
//! from a project-local name, so switching to `dev` in one checkout and walking
//! into another leaves you signed in as an account that checkout never named.
//! Every resolution therefore carries its source, and the caller says it out
//! loud.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// The file name looked for in a project, and upwards from it.
///
/// Undotted, matching every project file in the rbx-cli family
/// (`rbxplace.toml`, `rbxshop.toml`, and the rest): they are committed, read
/// and reviewed, so hiding them would work against their purpose. The personal
/// file keeps the leading dot that a home directory expects.
const PROJECT_FILE: &str = "rbxswitch.toml";
const GLOBAL_FILE: &str = ".rbxswitch.toml";

/// One `rbxswitch.toml` as written on disk.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct AliasFile {
    #[serde(default)]
    pub aliases: HashMap<String, u64>,
}

/// Which file an alias came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Project,
    Global,
}

impl Source {
    /// Wording for the line that reports a switch. Short on purpose: it sits
    /// at the end of a sentence the user reads every time.
    pub fn describe(self) -> &'static str {
        match self {
            Source::Project => "this project",
            Source::Global => "your personal aliases",
        }
    }
}

/// The merged view of the project file and the personal one.
#[derive(Debug, Default)]
pub struct Config {
    entries: HashMap<String, (u64, Source)>,
    project_path: Option<PathBuf>,
    global_path: Option<PathBuf>,
}

impl Config {
    /// Read both files, project first.
    ///
    /// Neither file existing is normal, not an error: aliases are a
    /// convenience over user ids and usernames, which always work.
    pub fn load() -> Result<Self> {
        let cwd = std::env::current_dir().context("Failed to read the working directory")?;
        Self::load_from(&cwd, home_dir().as_deref())
    }

    /// The testable half: no globals, no ambient working directory.
    pub fn load_from(start: &Path, home: Option<&Path>) -> Result<Self> {
        let project_path = find_upwards(start, PROJECT_FILE);
        let global_path = home.map(|h| h.join(GLOBAL_FILE)).filter(|p| p.exists());

        let mut config = Self {
            entries: HashMap::new(),
            project_path: project_path.clone(),
            global_path: global_path.clone(),
        };

        // Global first, so the project file overwrites it key by key.
        for (path, source) in [
            (global_path, Source::Global),
            (project_path, Source::Project),
        ] {
            let Some(path) = path else { continue };
            let file = read_file(&path)?;
            for (name, id) in file.aliases {
                config.entries.insert(name, (id, source));
            }
        }

        Ok(config)
    }

    /// The account this name points at, and which file said so.
    pub fn resolve_alias(&self, name: &str) -> Option<(String, Source)> {
        self.entries
            .get(name)
            .map(|(id, source)| (id.to_string(), *source))
    }

    /// Build a config in memory, for tests that care about resolution rather
    /// than about which file the answer came from.
    #[cfg(test)]
    pub fn with_alias(name: &str, id: u64, source: Source) -> Self {
        let mut config = Self::default();
        config.entries.insert(name.to_string(), (id, source));
        config
    }

    pub fn project_path(&self) -> Option<&Path> {
        self.project_path.as_deref()
    }

    pub fn global_path(&self) -> Option<&Path> {
        self.global_path.as_deref()
    }
}

/// The personal file's directory, or `None` when the platform will not say.
///
/// Deliberately not falling back to the working directory. That fallback used
/// to be here, and it silently turned the personal file into a per-directory
/// one depending on where the command was run from. A missing home directory
/// now means there is no personal file, which is true and says so.
fn home_dir() -> Option<PathBuf> {
    dirs::home_dir()
}

/// Walk up from `start` looking for `name`, the way a project root is found.
///
/// Running from a subdirectory of the checkout is the common case, not the
/// exception, so stopping at the working directory would make the project file
/// work only from the repository root.
fn find_upwards(start: &Path, name: &str) -> Option<PathBuf> {
    start
        .ancestors()
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

fn read_file(path: &Path) -> Result<AliasFile> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read aliases at {}", path.display()))?;

    toml::from_str(&content)
        .with_context(|| format!("Failed to parse aliases at {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write(dir: &Path, name: &str, body: &str) {
        fs::write(dir.join(name), body).expect("write fixture");
    }

    #[test]
    fn neither_file_present_is_not_an_error() {
        let project = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();

        let config = Config::load_from(project.path(), Some(home.path())).unwrap();

        assert!(config.resolve_alias("dev").is_none());
        assert!(config.project_path().is_none());
        assert!(config.global_path().is_none());
    }

    #[test]
    fn the_project_file_wins_over_the_personal_one() {
        let project = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();
        write(home.path(), GLOBAL_FILE, "[aliases]\ndev = 111\n");
        write(project.path(), PROJECT_FILE, "[aliases]\ndev = 222\n");

        let config = Config::load_from(project.path(), Some(home.path())).unwrap();

        assert_eq!(
            config.resolve_alias("dev"),
            Some(("222".to_string(), Source::Project)),
            "the project's name for an account must beat the personal one"
        );
    }

    #[test]
    fn the_two_files_merge_rather_than_replace_each_other() {
        let project = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();
        write(home.path(), GLOBAL_FILE, "[aliases]\nmain = 111\n");
        write(project.path(), PROJECT_FILE, "[aliases]\ndev = 222\n");

        let config = Config::load_from(project.path(), Some(home.path())).unwrap();

        assert_eq!(
            config.resolve_alias("main"),
            Some(("111".to_string(), Source::Global)),
            "a personal alias the project does not mention must survive"
        );
        assert_eq!(
            config.resolve_alias("dev"),
            Some(("222".to_string(), Source::Project))
        );
    }

    #[test]
    fn the_project_file_is_found_from_a_subdirectory() {
        let project = TempDir::new().unwrap();
        write(project.path(), PROJECT_FILE, "[aliases]\ndev = 222\n");
        let nested = project.path().join("src").join("client");
        fs::create_dir_all(&nested).unwrap();

        let config = Config::load_from(&nested, None).unwrap();

        assert_eq!(
            config.resolve_alias("dev"),
            Some(("222".to_string(), Source::Project)),
            "running from inside the checkout is the common case"
        );
    }

    #[test]
    fn no_home_directory_means_no_personal_file_and_not_a_local_one() {
        let project = TempDir::new().unwrap();
        write(project.path(), GLOBAL_FILE, "[aliases]\ndev = 999\n");

        let config = Config::load_from(project.path(), None).unwrap();

        assert!(
            config.resolve_alias("dev").is_none(),
            "a dotted file in the working directory must never be read as the personal one"
        );
    }

    #[test]
    fn a_malformed_file_names_itself() {
        let project = TempDir::new().unwrap();
        write(
            project.path(),
            PROJECT_FILE,
            "[aliases]\ndev = \"not a number\"\n",
        );

        let err = Config::load_from(project.path(), None).unwrap_err();

        assert!(
            format!("{err}").contains(PROJECT_FILE),
            "the error must name the file to edit, got: {err}"
        );
    }
}
