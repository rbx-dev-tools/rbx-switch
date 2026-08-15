# rbx-switch

Switch between Roblox Studio accounts from the command line.

`rbx-switch` reads the accounts signed into Roblox Studio and lets you switch between them without opening Studio. This is useful for headless workflows (e.g. switching before running [Lune](https://github.com/lune-org/lune) scripts that use `getAuthCookie`).

It used to be `rbx switch`, a subcommand of [`rbx-forge/rbx-cli`](https://github.com/rbx-forge/rbx-cli). It moved here because it shares no domain with that tool: `rbx` reconciles a repository with Roblox over Open Cloud, while this manipulates locally signed-in Studio accounts on one desktop. Nothing was ever released carrying `rbx switch`, so there is no old invocation to keep working.

## Install

```bash
rokit add rbx-forge/rbx-switch
```

Or take the Windows binary from the [releases page](https://github.com/rbx-forge/rbx-switch/releases), which ships a zip and a `SHA256SUMS` alongside it, or build from source:

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

An alias is a short name for an account:

```toml
[aliases]
dev = 9876543210
main = 1122334455
```

```bash
rbx-switch dev
```

Aliases come from two files, and the project one wins:

| File | Scope | Commit it? |
|------|-------|------------|
| `rbxswitch.toml` | This project. Looked for in the working directory and upwards, so it works from a subdirectory. | Yes |
| `~/.rbxswitch.toml` | You, everywhere. | It is in your home directory |

They merge key by key, so a personal `main` survives alongside a project's `dev`. Neither file has to exist: aliases are a convenience over usernames and user ids, which always work.

The project file is worth committing. It holds nothing but names pointing at **public** Roblox user ids, no secret of any kind, and its whole value is that everyone on the repository means the same account by `dev`.

The names differ by one character on purpose. Every project file in the [rbx-cli](https://github.com/rbx-forge/rbx-cli) family is undotted (`rbxplace.toml`, `rbxshop.toml`, and the rest) because those files are read and reviewed, while a home directory follows the opposite convention (`.gitconfig`, `.npmrc`). It also means a stray file in the wrong place is simply not read.

### One account for the whole machine

Studio has a single signed-in account, so a switch outlives the directory you made it in. If `dev` means different accounts in two checkouts, switching in one and then working in the other leaves you signed in as somebody that checkout never named, and the failure usually surfaces later as a permission error on a resource rather than as "wrong account".

Because of that, a switch made through an alias says which file chose it:

```
$ rbx-switch dev
Switched to DevAccount (9876543210)
  alias from this project - Studio stays on this account until you switch again
```

An alias pointing at an account that is no longer signed in is skipped rather than resolved, so the name falls through to the username lookup instead of failing on a dead id.

## How it works

Roblox Studio stores signed-in accounts and the active user ID in platform-specific credential stores:

- **Windows**: Accounts are listed in the Windows Registry (`LoggedInUsersStore`), and the active user ID is stored in Windows Credential Manager (`RobloxStudioAuthuserid`). Tools like [rbx_cookie](https://github.com/blake-mealey/mantle/tree/main/rbx_cookie) read this credential to determine which `.ROBLOSECURITY` cookie to use.

- **macOS**: The storage locations are known (`defaults` for the account list, the app's binary cookie store for the active user id) but the implementation is a stub that returns "not yet tested" from every method. Finishing it needs a Mac with Studio installed to verify both locations against a real install. Contributions welcome.

## Platform support

| Platform | Status |
|----------|--------|
| Windows | Supported and tested in CI |
| macOS | Stub. The code paths exist and are unverified. |
| Linux | Roblox does not ship Studio for Linux, so there is no account list to read. |

CI runs on Windows only. It is the one platform where the credential store this tool talks to exists, so a job anywhere else would exercise the argument parser and report green on the day the real calls broke. A macOS runner, billed at ten times the rate, would test a stub.

## License

[MPL-2.0](./LICENSE).
