# rbx-switch

Switch between Roblox Studio accounts from the command line.

`rbx-switch` reads the accounts signed into Roblox Studio and lets you switch between them without opening Studio. This is useful for headless workflows (e.g. switching before running [Lune](https://github.com/lune-org/lune) scripts that use `getAuthCookie`).

It used to be `rbx switch`, a subcommand of [`rbx-forge/rbx-cli`](https://github.com/rbx-forge/rbx-cli). It moved here because it shares no domain with that tool: `rbx` reconciles a repository with Roblox over Open Cloud, while this manipulates locally signed-in Studio accounts on one desktop. Nothing was ever released carrying `rbx switch`, so there is no old invocation to keep working.

## Install

Prebuilt Windows binary from the [releases page](https://github.com/rbx-forge/rbx-switch/releases), or from source:

```bash
cargo install --git https://github.com/rbx-forge/rbx-switch
```

## Usage

```bash
rbx-switch list              # List all signed-in accounts (* = active)
rbx-switch current           # Show the currently active account
rbx-switch use <name>        # Switch by username, user ID, or alias
rbx-switch <name>            # Shorthand for `rbx-switch use <name>`
rbx-switch                   # Interactive account picker
```

Two flags apply everywhere, before or after the subcommand:

| Flag | Effect |
|------|--------|
| `--no-color` | Never colour the output. `NO_COLOR` in the environment does the same without the flag. |
| `--non-interactive` | Fail instead of opening the picker, so a script with a missing account name errors out rather than hanging. |

### Example

```
$ rbx-switch list
  BuilderBot (1234567890)
  DevAccount (9876543210) *
  MainAccount (1122334455)

$ rbx-switch use MainAccount
Switched to MainAccount (1122334455)
```

## Aliases

Create `~/.rbxswitch.toml` to define short aliases:

```toml
[aliases]
dev = 9876543210
main = 1122334455
```

Then use them directly:

```bash
rbx-switch dev
```

An alias pointing at an account that is no longer signed in is skipped rather than resolved, so the name falls through to the username lookup instead of failing on a dead id.

## How it works

Roblox Studio stores signed-in accounts and the active user ID in platform-specific credential stores:

- **Windows**: Accounts are listed in the Windows Registry (`LoggedInUsersStore`), and the active user ID is stored in Windows Credential Manager (`RobloxStudioAuthuserid`). Tools like [rbx_cookie](https://github.com/blake-mealey/mantle/tree/main/rbx_cookie) read this credential to determine which `.ROBLOSECURITY` cookie to use.

- **macOS**: Not yet implemented - contributions welcome!

## Platform support

| Platform | Status |
|----------|--------|
| Windows | Supported |
| macOS | Stub (not yet tested) |
| Linux | Not yet supported (Studio runs via [Vinegar](https://vinegarhq.org/)) |

CI runs on Windows only, for the same reason: it is the one platform where the credential store this tool talks to actually exists.

## License

[MPL-2.0](./LICENSE).
