# Changelog

## [Unreleased]

### Added

- **`--player` keeps a backup of the Player's cookie store**, as `RobloxCookies.dat.rbx-account.bak` beside it: the store as it was before the latest switch. It is written before the sign-in ticket is requested or a Player closed, and a backup that cannot be written stops the switch with nothing changed. With the Player closed, copying it back over `RobloxCookies.dat` undoes that switch.

### Changed

- **`--player` no longer opens the Player.** Like a Studio switch, it changes the account and launches nothing: the closed Player starts signed in as the account next time. The sign-in ticket is redeemed by `rbx-account` itself, for a session of the Player's own, and written into the Player's cookie store; nothing else in the store changes.
- **`--force` closes an open Player instead of restarting it**, game in progress included, and does not reopen it.

### Fixed

- **`--player` checks the Player's cookie store before anything changes.** A store that is missing, cannot be decrypted, is not in the expected format or is signed out is refused before a sign-in ticket is requested and before `--force` closes the Player. If the store changes after that check and can no longer be read, for instance left torn by a Player closed while writing it, the session goes into the store as it was checked, and the switch says so.
- **`--player` keeps every key of the Player's cookie store file**, the ones it does not use included, instead of dropping them. The new store and its backup are each written to a temp file of their own, created for that write alone and flushed to disk, then renamed into place, so two runs at once cannot leave a half-written store. A temp file left by a run that died holds a whole encrypted session, so each switch removes the ones older than a minute, and nothing else.
- **`--force` no longer reports a Player it could not close as closed.** When Windows refuses to open the Player's process, for instance a Player running as administrator, the switch stops there with the reason, before anything is written. "Closed the Roblox Player" is printed only when a Player was actually closed.
- **`--force` closes every open Player or none.** Every running Player is opened and identified before the sign-in ticket is requested, so one that cannot be closed refuses the switch with nothing closed and no session issued. A Player that is already exiting counts as closed.
- **`--force` closes only the Roblox Player.** Before terminating a process, it checks that the process still runs `RobloxPlayerBeta.exe`: one that took the id of a Player that exited is left running, and named.
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
