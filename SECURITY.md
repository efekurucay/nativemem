# Security

## What the tool touches

`nativemem` reads and writes files in the agent home directories (Claude Code, Codex, OpenCode, Pi)
and in its own state directory (`~/.nativemem` by default: ledger, backups, `compat.json`). Those
files can hold private notes and chat transcripts. The tool never transmits anything: it makes no
network requests and has no telemetry. Secrets and session bodies are never printed, and the
ledger stores ids, hashes and paths, not entry text. Every overwrite or delete is backed up first,
and nothing is written inside a git repository. Details: `wiki/architecture/safety.md`.

On macOS, `schedule install` writes a launchd job in `~/Library/LaunchAgents` that runs a copy of
the binary; `schedule uninstall` removes it.

## Reporting a vulnerability

Report it privately through a GitHub security advisory:
<https://github.com/efekurucay/nativemem/security/advisories/new>. Do not open a public issue.
Include the version (`nativemem --version`), the platform and the steps to reproduce, with
anything personal removed. Reports about writes outside the agent homes and the state directory,
about printed secrets or session text, and about data loss without a backup are in scope.

## Supported versions

Fixes go into the latest release.
