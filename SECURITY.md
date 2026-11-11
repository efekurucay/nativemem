# Security

## What the tool touches

File sync reads and writes the native stores of Claude Code, Codex, OpenCode and Pi, plus its own
state directory (`~/.nativemem` by default: ledger, backups, `compat.json`). Project skills also write
the agent-specific skill directory inside that project. Codex and OpenCode session writers update
only their own replica rows under compatibility checks. These files and backups can hold private
notes and transcripts; treat them as sensitive. Every overwrite or delete is backed up first.

The file-sync engine makes no network requests or model calls and has no telemetry. The optional
MCP hub connects to the servers and OAuth endpoints you configure, keeps secrets in the OS keychain
and writes the one `nativemem` entry in agent MCP configurations. Subagents run with their harness's
permissions, credentials and model provider. Configure only servers and commands you trust.

The ledger stores IDs, hashes and paths, not entry text. Listing commands omit session bodies and
skill file contents; an explicit `session show` or `skill show` prints the selected content.
Details: [safety rules](wiki/architecture/safety.md) and [hub behavior](wiki/architecture/hub.md).

On macOS, `schedule install` writes a launchd job in `~/Library/LaunchAgents` that runs a copy of
the binary; `schedule uninstall` removes it.

## Reporting a vulnerability

Report it privately through a GitHub security advisory:
<https://github.com/efekurucay/nativemem/security/advisories/new>. Do not open a public issue.
Include the version (`nativemem --version`), the platform and the steps to reproduce, with
anything personal removed. Unexpected writes outside the declared stores and project skill
directories, secret leakage, cross-project contamination and data loss without a backup are in scope.

## Public data boundaries

Only synthetic fixtures and window-only screenshots belong in this repository. Never commit real
agent homes, session exports, credential files or desktop captures containing other applications.
Public release archives contain the built software, licenses and installation information—not local
agent state, development logs or repository backups. Redact issue reports before posting them.

If a private file reaches git, deleting it in a later commit does not remove its history. Keep the
repository private while assessing exposure. Rewrite affected refs with a verified private backup;
GitHub-managed PR refs and cached commit views can require help from GitHub Support. Rotate any
exposed credential even after removing it from history.

## Supported versions

Fixes go into the latest release.
