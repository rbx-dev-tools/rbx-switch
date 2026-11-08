# rbx-account

Switch the Roblox Studio account that is signed in, from the command line, without opening Studio. Useful before a script that reads Studio's session, such as a [Lune](https://github.com/lune-org/lune) script using `getAuthCookie`, or any tool that borrows Studio's cookie.

```bash
rbx-account list              # signed-in accounts, * marks the active one
rbx-account current           # the active account
rbx-account use <name>        # by username, user id or alias
rbx-account <name>            # the same
rbx-account                   # pick from a list
rbx-account <name> --player   # switch the Roblox Player instead
```

## Requirements

- **Windows.** macOS is an untested stub, and Studio does not exist on Linux.
- Every account you want to switch to must have signed in to Studio once. `rbx-account` switches between them; it signs nobody in.

## Install

With [mise](https://mise.jdx.dev):

```bash
mise use --pin github:rbx-forge/rbx-account
```

Or with [Rokit](https://github.com/rojo-rbx/rokit):

```bash
rokit add rbx-forge/rbx-account
```

Either one pins the latest release in your project's config.

Or take the Windows binary from the [releases page](https://github.com/rbx-forge/rbx-account/releases), which ships a zip and a `SHA256SUMS`, or build from source:

```bash
cargo install --git https://github.com/rbx-forge/rbx-account --locked
```

## Options

Accepted before or after the subcommand:

| Flag | Effect |
|------|--------|
| `--no-color` | No colour. `NO_COLOR` does the same |
| `--non-interactive` | Fail instead of opening the picker, so a script never hangs |
| `--json` | JSON from `list` and `current`. Implies `--no-color` |

```bash
$ rbx-account list --json
{"accounts":[{"user_id":"1234567890","username":"MainAccount","active":true}]}

$ rbx-account current --json
{"account":{"user_id":"1234567890","username":"MainAccount"}}
```

`current` gives `{"account":null}` when Studio has no active account.

## Aliases

```toml
# rbxaccount.toml
[aliases]
qa = 9876543210
assets = 1122334455
```

```bash
rbx-account qa
```

| File | For |
|------|-----|
| `rbxaccount.toml` | Accounts the **project** uses (a group's account, a QA account). Found in the working directory or any parent. Safe to commit: it holds public user ids, never a credential |
| `~/.rbxaccount.toml` | Your own accounts |

The two merge, and the project file wins on a shared name. An alias pointing at an account no longer signed in is ignored, and the name is tried as a username.

**One account for the whole machine.** Studio has one signed-in account, so a switch made in one checkout applies everywhere. A switch through an alias says which file chose it, so a wrong account shows up here rather than later as a permission error.

## The Roblox Player

`--player` switches the Player rather than Studio, with the same accounts and aliases:

```bash
rbx-account qa --player              # the Player, signed in as qa
rbx-account qa --studio --player     # both apps
rbx-account --player                 # pick from a list
```

Any account signed in to Studio can be switched to: the Player has no account list of its own here.

**It restarts the Player.** Switching starts a new Player on its home screen, signed in as the account, and the one already open exits a few seconds later, leaving any game in progress. So `--player` is refused while a Player is open; close it, or pass `--force` to switch anyway. The Player stays on the account afterwards.

**Wait before joining a game.** `rbx-account qa --player && <something that joins a game>` races: the join reaches the old Player just before it is closed. Join once the new Player shows its home screen.

`list` and `current` stay about Studio. The Player keeps no account this tool can read back, so `current --player` is refused rather than answered with Studio's account.

Windows only.

## How it works

On Windows, Studio lists signed-in accounts in the registry (`LoggedInUsersStore`) and keeps the active user id in Credential Manager (`RobloxStudioAuthuserid`). `rbx-account` rewrites the latter. On macOS the locations are known but unverified; contributions from someone with a Mac and Studio are welcome.

For `--player`, it reads the account's Studio session from Credential Manager and sends it to `auth.roblox.com` to get a single-use sign-in ticket, the one the website gets when you press Play. It then opens a `roblox-player:` link carrying that ticket. The session goes to `auth.roblox.com` and nowhere else, and is never printed. This is the only network call the tool makes, and only `--player` makes it.

## License

[MPL-2.0](./LICENSE). A community tool, not affiliated with Roblox Corporation.
