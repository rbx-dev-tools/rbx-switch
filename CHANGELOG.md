# Changelog

## [Unreleased]

### Added

- **`--player`** switches the Roblox Player instead of Studio, with the same accounts and aliases: `rbx-account qa --player`. Pass `--studio` too to switch both. It asks `auth.roblox.com` for a sign-in ticket with the account's Studio session and starts the Player with it, so any account signed in to Studio works. Windows only.
- **`--force`**: `--player` restarts the Player, closing an open one and any game in progress, so it is refused while a Player is open unless `--force` is passed.

## [0.3.0] - 2026-11-08

### Changed

- **Renamed from rbx-switch to rbx-account**: the repository, the binary and the crate. "Switch" alone did not say what is switched, in a set of tools where envs are switched too.
- **The alias files are `rbxaccount.toml` and `~/.rbxaccount.toml`.** Rename an existing `rbxswitch.toml` or `~/.rbxswitch.toml`: the old names are no longer read.
- The repository moved to the rbx-forge organisation.

Earlier releases are described on the [releases page](https://github.com/rbx-forge/rbx-account/releases).
