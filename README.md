# rbx-account

Switch the Roblox Studio account that is signed in, from the command line, without opening Studio. Useful before a script that reads Studio's session, such as a [Lune](https://github.com/lune-org/lune) script using `getAuthCookie`, or any tool that borrows Studio's cookie.

```bash
rbx-account list              # signed-in accounts, * marks the active one
rbx-account current           # the active account
rbx-account use <name>        # by username, user id or alias
rbx-account <name>            # the same
rbx-account                   # pick from a list
```

## Requirements

- **Windows.** macOS is an untested stub, and Studio does not exist on Linux.
- Every account you want to switch to must have signed in to Studio once. `rbx-account` switches between them; it signs nobody in.

## Install

With [mise](https://mise.jdx.dev) or [Rokit](https://github.com/rojo-rbx/rokit), either of which pins the latest release in your project's config:

```bash
mise use --pin github:rbx-forge/rbx-account
rokit add rbx-forge/rbx-account   # or, with Rokit
```

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

## How it works

On Windows, Studio lists signed-in accounts in the registry (`LoggedInUsersStore`) and keeps the active user id in Credential Manager (`RobloxStudioAuthuserid`). `rbx-account` rewrites the latter. On macOS the locations are known but unverified; contributions from someone with a Mac and Studio are welcome.

## License

[MPL-2.0](./LICENSE). A community tool, not affiliated with Roblox Corporation.
