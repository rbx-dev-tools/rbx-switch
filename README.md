# rbx-switch

Switch between Roblox Studio accounts from the command line.

`rbx-switch` reads the accounts signed into Roblox Studio and lets you switch between them without opening Studio. This is useful for headless workflows (e.g. switching before running [Lune](https://github.com/lune-org/lune) scripts that use `getAuthCookie`).

If you saw this as `rbx switch`: it was a subcommand of `rbx-forge/rbx-cli` and now lives here, as `rbx-switch`. Nothing was ever released carrying the old spelling, so there is no version of it still working anywhere.

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
# rbxswitch.toml, committed with the project
[aliases]
qa = 9876543210
assets = 1122334455
```

```bash
rbx-switch qa
```

Aliases come from two files, and the project one wins:

| File | For |
|------|-----|
| `rbxswitch.toml` | Accounts the **project** owns: the account behind the group, an assets account, a QA account. Looked for in the working directory and upwards, so it works from a subdirectory. |
| `~/.rbxswitch.toml` | Accounts **you** own. |

They merge key by key, so your personal `main` survives alongside the project's `qa`. Neither file has to exist: aliases are a convenience over usernames and user ids, which always work.

The split is about whose account it is, not about precedence. A shared purpose-built account means the same thing to everyone on the repository, so naming it once in a committed file saves every teammate from rediscovering the id. Your own account means nothing to them: put it in your personal file, where it follows you across projects instead of sitting in someone else's checkout.

Committing the project file is safe in itself, whatever you put in it. It holds names pointing at public Roblox user ids, the kind of number anyone can read off a profile, and never a credential of any kind.

When both files name the same alias, the project wins, the way a repository's `git config` beats your global one. That works because the two files are meant to hold different accounts: if you find yourself wanting to override a project alias with your own account, that alias was a personal one written in the wrong file.

The leading dot on the personal file is the only difference between the two names. It follows what each location expects: a committed file stays visible because people read and review it, a home-directory file hides because nobody wants it in every listing. It also means a copy left in the wrong place is simply not read.

### One account for the whole machine

Studio has a single signed-in account, so a switch outlives the directory you made it in. If `qa` means different accounts in two checkouts, switching in one and then working in the other leaves you signed in as somebody that checkout never named, and the failure usually surfaces later as a permission error on a resource rather than as "wrong account".

Because of that, a switch made through an alias says which file chose it:

```
$ rbx-switch qa
Switched to QaAccount (9876543210)
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
