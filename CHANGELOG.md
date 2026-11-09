# Changelog

## [Unreleased]

### Changed

- **`--player` no longer opens the Player.** Like a Studio switch, it changes the account and launches nothing: the closed Player starts signed in as the account next time. The sign-in ticket is redeemed by `rbx-account` itself, for a session of the Player's own, and written into the Player's cookie store; nothing else in the store changes. `--force` is how to switch an open Player: it starts a new one signed in, as `--player` did before.

## [0.4.0] - 2026-11-08

### Added

- **`--player`** switches the Roblox Player instead of Studio, with the same accounts and aliases: `rbx-account qa --player`. Pass `--studio` too to switch both. It asks `auth.roblox.com` for a sign-in ticket with the account's Studio session and starts the Player with it, so any account signed in to Studio works. Windows only.
- **`--force`**: `--player` restarts the Player, closing an open one and any game in progress, so it is refused while a Player is open unless `--force` is passed.
- Release archives carry a build provenance attestation: `gh attestation verify <zip> --repo rbx-forge/rbx-account`.

## [0.3.0] - 2026-11-08

### Changed

- **Renamed from rbx-switch to rbx-account**: the repository, the binary and the crate. "Switch" alone did not say what is switched, in a set of tools where envs are switched too.
- **The alias files are `rbxaccount.toml` and `~/.rbxaccount.toml`.** Rename an existing `rbxswitch.toml` or `~/.rbxswitch.toml`: the old names are no longer read.
- The repository moved to the rbx-forge organisation.

Earlier releases are described on the [releases page](https://github.com/rbx-forge/rbx-account/releases).
