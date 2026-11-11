# rbx-account

Switch the Roblox account that Studio, or the Player with `--player`, is signed in as, from the command line, without opening either. Useful before a script that reads Studio's session, such as a [Lune](https://github.com/lune-org/lune) script using `getAuthCookie`, or any tool that borrows Studio's cookie.

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

**Nothing opens.** Like a Studio switch, `--player` changes the account and launches nothing: the Player starts signed in as it next time you open it, and stays on it.

**The Player has to be closed**, because an open one writes its sign-in back when it exits. While it is open, `--player` is refused; close it, or pass `--force` to have it closed for you, game in progress included. It is not reopened.

`list` and `current` stay about Studio. The Player keeps no account this tool can read back, so `current --player` is refused rather than answered with Studio's account.

Windows only.

## How it works

On Windows, Studio lists signed-in accounts in the registry (`LoggedInUsersStore`) and keeps the active user id in Credential Manager (`RobloxStudioAuthuserid`). `rbx-account` rewrites the latter. On macOS the locations are known but unverified; contributions from someone with a Mac and Studio are welcome.

For `--player`, it first reads and checks the Player's cookie store, so a store it could not write into refuses the switch before anything is sent or closed. It then reads the account's Studio session from Credential Manager and sends it to `auth.roblox.com` to get a single-use sign-in ticket, the one the website gets when you press Play. It redeems that ticket for a session of the Player's own, checks with `users.roblox.com` that the session is the account's, and writes it into the Player's cookie store (`%LOCALAPPDATA%\Roblox\LocalStorage\RobloxCookies.dat`, encrypted for your Windows user), changing nothing else in it. Sessions go to Roblox's own hosts over HTTPS, never through a redirect, and are never printed. These are the only network calls the tool makes, and only `--player` makes them.

The store is replaced in one step, through a file beside it, and the store as it was before the switch is kept next to it as `RobloxCookies.dat.rbx-account.bak`. That backup is encrypted the same way and holds the Player's previous session, so treat it like the store itself; each switch replaces it. With the Player closed, copying it back over `RobloxCookies.dat` undoes the switch.

## License

[MPL-2.0](./LICENSE). A community tool, not affiliated with Roblox Corporation.
