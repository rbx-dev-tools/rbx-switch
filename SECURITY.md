# Security policy

`rbx-account` works next to Roblox session cookies, whole account sessions.
It rewrites the active user in Credential Manager, and with `--player` it reads
an account's Studio session, trades it for a sign-in ticket and writes a new
session into the Player's cookie store. That makes the threat model worth
stating.

## Reporting a vulnerability

Report it privately, through GitHub: on this repository, open the **Security**
tab and choose **Report a vulnerability**, or go straight to
<https://github.com/rbx-forge/rbx-account/security/advisories/new>.

Do not open a public issue for a vulnerability. Do not paste a real cookie,
ticket or Credential Manager entry into a report: a redacted reproduction is
always enough.

What helps: the `rbx-account --version`, the command you ran, what you
expected, what happened, and why it is a security problem rather than a bug.
A proof of concept is welcome but never required.

**What to expect.** This is a solo-maintained project. An acknowledgement
within a week is the realistic commitment; a fix lands as fast as the severity
warrants. You will be credited in the advisory and the changelog unless you
ask otherwise. There is no bounty program.

## Supported versions

Only the latest release. At 0.x, fixes go out as a new release rather than
being backported.

## What counts

In scope:

- A session or a ticket printed, logged, written anywhere other than the
  Player's cookie store, or sent anywhere other than the Roblox hosts the
  README names, including through a redirect.
- The Player's cookie store left readable by anyone but your Windows user, or
  changed beyond the one session `--player` writes.
- A switch that signs Studio or the Player in as an account other than the one
  asked for.
- A credential reaching `rbxaccount.toml`, which is meant to be committed.
- Dependency vulnerabilities with a plausible path to any of the above.

Out of scope:

- Anything requiring an attacker who already runs code as your Windows user:
  that user can read Credential Manager and the cookie store without this
  tool.
- Rate limits and Roblox-side behavior. Report those to Roblox.

## Verifying a download

Each release archive carries a build provenance attestation, a signed
statement of which workflow run and commit produced it:

```sh
gh attestation verify rbx-account-v0.0.0-x86_64-pc-windows-msvc.zip --repo rbx-forge/rbx-account
```
