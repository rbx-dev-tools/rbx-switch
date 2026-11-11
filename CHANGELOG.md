# Changelog

## [Unreleased]

### Added

- **`--player` keeps a backup of the Player's cookie store**, as `RobloxCookies.dat.rbx-account.bak` beside it: the store as it was before the latest switch. With the Player closed, copying it back over `RobloxCookies.dat` undoes that switch.

### Changed

- **`--player` no longer opens the Player.** Like a Studio switch, it changes the account and launches nothing: the closed Player starts signed in as the account next time. The sign-in ticket is redeemed by `rbx-account` itself, for a session of the Player's own, and written into the Player's cookie store; nothing else in the store changes.
- **`--force` closes an open Player instead of restarting it**, game in progress included, and does not reopen it.

### Fixed

- **`--player` checks the Player's cookie store before anything else.** A store that is missing, cannot be decrypted, is not in the expected format or is signed out is now refused before a sign-in ticket is requested and before `--force` closes the Player. It used to be read last, after the Player was closed.
- **`--player` keeps every key of the Player's cookie store file**, the ones it does not use included, instead of dropping them. The new store is flushed to disk before it replaces the old one.
- **`--force` no longer reports a Player it could not close as closed.** When Windows refuses to open the Player's process, for instance a Player running as administrator, the switch stops there with the reason, before anything is written. "Closed the Roblox Player" is printed only when a Player was actually closed.
- **`--player` gives up on an unresponsive Roblox server** instead of waiting forever: 10 seconds to connect, 20 seconds for any one read, 30 seconds for a whole request.

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
