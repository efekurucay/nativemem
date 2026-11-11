# File structure

## Purpose

The real tree of the repo and what each crate, module and folder owns, so a change lands in the
right place.

## Scope

Tracked source, docs, scripts, CI configuration and wiki. Build output (`target/`, `apps/macos/.build/`,
`apps/macos/build/`, `dist/`) and the local scratch folder `gitdex/` are ignored by `.gitignore` and not described.

## Tree

```
nativemem-sync/
├── Cargo.toml                    workspace: crates/core, crates/hub, crates/cli; version and package metadata
├── LICENSE                       MIT
├── CONTRIBUTING.md · SECURITY.md contribution rules; private vulnerability reporting
├── CLAUDE.md                     entry point for coding agents (AGENTS.md is a symlink to it)
├── README.md                     user-facing overview, usage, support table
├── docs/
│   ├── design.md                 original design and build plan
│   ├── desktop.md                desktop app spec and the CLI JSON contract
│   └── images/                   synthetic app screenshots: overview.png, memory-detail.png, skills.png
├── site/                         static landing page; no build step or runtime dependencies
│   ├── index.html · style.css · app.js  product page, responsive styling, accessible tabs and copying
│   ├── render-icon.mjs            renders assets/mark.svg into the macOS source AppIcon.png
│   ├── assets/                   mark.svg, social.png, synthetic app screenshots (WebP), self-hosted OFL fonts
│   │   └── agents/               Claude, Codex, OpenCode and Pi integration marks; sources in site/README.md
│   ├── package.json · package-lock.json  browser-test dependencies only
│   ├── playwright.config.js · tests/site.spec.js  navigation, logos, tabs, clipboard, responsive and axe checks
│   ├── tests/server.py           local HTTP fixture with an OS-sized connection backlog for parallel browser loads
│   └── README.md                 local preview, browser tests, Pages deployment
├── scripts/
│   ├── release.sh                local release: checks, version bump, macOS artifacts, notes, commit, tag
│   ├── demo-home.sh              synthetic agent homes with generic memory items and skills for trying the CLI and app
│   └── benchmark-startup.py      isolated small/large fixtures, CLI startup measurements and paired binary comparison
├── .github/
│   ├── workflows/                ci.yml (Linux checks), site.yml (browser checks + Pages), release.yml (Linux binaries, release), portability.yml (manual Windows build)
│   ├── ISSUE_TEMPLATE/           bug_report, agent_update, config
│   ├── PULL_REQUEST_TEMPLATE.md
│   └── dependabot.yml            weekly grouped cargo and actions updates
├── crates/
│   ├── core/                     library `nativemem-core`: no agent format knowledge in the engine
│   │   └── src/
│   │       ├── lib.rs            module list
│   │       ├── model.rs          AgentId, Scope, Kind, Item, ItemKey, Observed, ScanReport, the Adapter trait, content_hash
│   │       ├── engine.rs         plan (pure) and apply: three-way decision, session planner, mass-delete guard
│   │       ├── ledger.rs         SQLite ledger: items and replicas tables, writer revision column
│   │       ├── fsx.rs            atomic_write, BackupSession, backup layout, backup_dir for skill directories
│   │       ├── session.rs        neutral session model, tool rendering, titles, window, markers, derived ids
│   │       ├── compat.rs         Check, AgentReport, Status, compat.json state, assess, prompt, notifications
│   │       ├── probe.rs          read-only helpers for probes: tails, newest files, executable lookup
│   │       ├── slug.rs           slugify
│   │       ├── error.rs          Error enum (Io, Parse, Ledger, Unsupported, MassDelete)
│   │       ├── testkit.rs        fixture builders: ClaudeHome, CodexHome, PiHome, OpencodeData (temp dirs only; `testkit_opencode_schema.sql` is the real structure of OpenCode's session tables)
│   │       └── adapters/
│   │           ├── mod.rs        adapter module list
│   │           ├── claude.rs     Claude Code: project memory, CLAUDE.md sections, session replicas
│   │           ├── claude_sessions.rs  Claude transcript reader and writer
│   │           ├── codex.rs      Codex: generated-memory originals, replica store, AGENTS.md, rollouts
│   │           ├── codex_sessions.rs   Codex rollout reader (originals, replica detection, automated markers)
│   │           ├── codex_threads.rs    `threads` row writer for replica rollouts, database backup and retention
│   │           ├── sqlite_guard.rs     shared by the Codex and OpenCode row writers: busy timeout, busy-or-failed, online backup once per run, 5 kept
│   │           ├── opencode_sessions.rs  OpenCode session reader and writer for `opencode.db` (rows, marker, verify)
│   │           ├── opencode_project.rs   OpenCode project id and worktree for a directory (SHA-1, git rules)
│   │           ├── codex_rollout.rs    Codex rollout writer shaped like Codex's own importer output
│   │           ├── codex_imports.rs    Codex importer ledger (`external_agent_session_imports.json`)
│   │           ├── claude_probes.rs    Claude compatibility probes and their expected constants
│   │           ├── codex_probes.rs     Codex probes: state database, rollouts, import ledger
│   │           ├── pi_probes.rs        Pi probes: session header, entry types, directory encoding
│   │           ├── opencode_probes.rs  OpenCode probes: config directory, AGENTS.md, session tables, installed version
│   │           ├── pi.rs         Pi: AGENTS.md through agents_file, session reader and writer
│   │           ├── agents_file.rs      generic adapter for agents that only read a global AGENTS.md (OpenCode, Pi)
│   │           ├── skills.rs     skill directories: manifest hash, staged tree copy, project path index, per-project backups, visible roots, layout probe
│   │           ├── memory_store.rs     item-file store (originals and replicas) plus routing block, shared by Codex, OpenCode, Pi
│   │           └── sections.rs   instruction-section parser, managed regions, byte-preserving edits, markers
│   │   └── tests/                probes.rs (probes, drift, blocked kinds), opencode_sessions.rs (OpenCode reader, writer, database safety, probes, engine flow), skills.rs (skill mirroring over four temp homes)
│   ├── hub/                      library `nativemem-hub`: the MCP hub; not a sync kind, no engine or ledger
│   │   ├── src/
│   │   │   ├── lib.rs            module list
│   │   │   ├── config.rs         `hub.toml`: ServerSpec, HubConfig, validation, load, backed-up atomic save
│   │   │   ├── secrets.rs        SecretStore trait, KeychainStore, FileStore (tests), `secret:<id>` references
│   │   │   ├── connect.rs        connect to one upstream (stdio child or streamable HTTP, bearer for OAuth servers), secret resolution, scrubbing, `mcp test`
│   │   │   ├── serve.rs          HubServer: tool naming, forwarding, `nativemem_status`, subagent tools, stdio serving
│   │   │   ├── subagents.rs      `subagents.toml`: SubagentSpec, Harness, SubagentConfig, validation, load, backed-up atomic save
│   │   │   ├── harness.rs        the headless command of each harness, executable lookup, answer parsing
│   │   │   ├── runner.rs         Runner: one run with timeout and process-group kill, 4 at a time, depth guard, answer cut
│   │   │   ├── import.rs         read the agents' MCP entries, plan and apply the import
│   │   │   ├── oauth.rs          OAuth sign-in (PKCE, discovery, registration, loopback redirect), token store, refresh lock, logout
│   │   │   ├── register.rs       register the hub in each agent: the `nativemem` MCP entry (Claude, Codex, OpenCode, Pi), install, uninstall, status
│   │   │   └── bin/fake_mcp.rs   `nativemem-fake-mcp`, a tiny stdio MCP server for tests
│   │   └── tests/                hub.rs (proxy, failures, secrets, HTTP headers), import.rs (four fake agent homes), oauth.rs (sign-in, refresh, needs_login over a fake authorization server), register.rs (the entry in four fake agent homes), subagents.rs (every harness against fake scripts, timeout, depth, cut, concurrency, the hub tools), common/fake_oauth.rs (the fake server, also used by the CLI tests)
│   └── cli/                      binary `nativemem`
│       ├── src/
│       │   ├── main.rs           clap CLI, root resolution, lock, snapshot/status/items/sync/delete/add/edit, JSON output
│       │   ├── doctor.rs         `doctor`, compatibility reports, warnings, notification delivery
│       │   ├── mcp.rs            `mcp` subcommands over the hub crate
│       │   ├── subagent.rs       `subagent` subcommands over the hub crate
│       │   └── schedule.rs       launchd scheduling (macOS only)
│       └── tests/                cli.rs (end-to-end CLI tests over temp homes), doctor.rs (doctor, gating, notifications), opencode_sessions.rs (drift, data-home resolution), mcp.rs (hub commands, sign-in, registration and `mcp serve` over temp roots), subagent.rs (subagent commands and the subagent tools of `mcp serve` over temp roots and fake harnesses)
├── apps/macos/                   Swift package, macOS 14+, no third-party dependencies
│   ├── Package.swift
│   ├── Sources/Nativemem/        app entry point (`NativememApp.swift`)
│   ├── Sources/NativememKit/     app logic and views
│   │   ├── SkillTextView.swift   read-only SKILL.md of a skill
│   │   ├── SessionIndex.swift, SessionDetailView.swift  sessions per project folder, read-only transcript
│   │   ├── NewEntry.swift, NewEntrySheet.swift  the New sheet: draft and validation, the form
│   │   ├── McpContract.swift, McpCLI.swift  Codable types of the `mcp` documents, server state and badge count; the `mcp` commands
│   │   ├── McpModel.swift, McpDraft.swift  the MCP & Integrations screen state and import flow; the Add server / Edit draft
│   │   ├── McpView.swift, McpServerSheet.swift, McpImportSheet.swift  agents box, server list and detail; the Add / Edit sheet; the import sheet
│   │   ├── SubagentContract.swift, SubagentCLI.swift  Codable types of the `subagent` documents and run results; the `subagent` commands
│   │   ├── SubagentModel.swift, SubagentDraft.swift  the Subagents screen state and its Try it run; the New subagent / Edit draft
│   │   ├── SubagentView.swift, SubagentSheet.swift  list, detail and Try it box; the New / Edit sheet
│   │   ├── NativememCLI.swift    finds and runs the binary, decodes JSON
│   │   ├── Contract.swift        Codable types mirroring the CLI JSON contract
│   │   ├── AppModel.swift        observable state, grouping, copy folding
│   │   ├── MemoryIndex.swift     agent display names, project keys, grouping of items
│   │   ├── ProjectTree.swift     folder tree built from project paths
│   │   ├── SyncSession.swift     preview then apply flow
│   │   ├── BodyText.swift, MarkdownBlocks.swift  body rendering
│   │   └── SidebarView, ItemListView, ItemDetailView, OverviewView, SyncSheet, ContentView
│   ├── Tests/NativememKitTests/  contract, doctor, index, session, tree, sectioning, body parser, sync headline, new entry, MCP contract, server state and server draft tests
│   │   ├── LoadingTests.swift    fake-only startup, lazy loading, diagnostics, retry, cancellation and superseded reload tests
│   │   └── Fixtures/             one JSON file per CLI document shape
│   ├── Resources/AppIcon.png      1024px desktop icon generated from site/assets/mark.svg
│   └── scripts/                  bundle.sh builds `build/nativemem.app` (`VERSION`, `UNIVERSAL`, `NATIVEMEM_CLI`, `CODESIGN_IDENTITY`); dmg.sh packages an app with an Applications shortcut
└── wiki/                         this knowledge base
    ├── README.md · CLAUDE.md · WIKI-CREATOR.md
    ├── architecture/             ci, compatibility, context-quality, file-structure, hub, overview, safety
    ├── kinds/                    memory, instruction-sections, sessions, skills
    ├── agents/                   claude-code, codex, pi, opencode, hermes
    ├── runbooks/                 agent-update-action-required
    ├── guides/                   add-an-agent, add-a-resource-kind, release, verify-with-real-agents
    ├── decisions/                one short decision record per file
    └── _templates/               PAGE.md, AGENT.md
```

## Ownership

| Concern | Owner |
|---|---|
| What an item is, hashing, the adapter contract | `crates/core/src/model.rs` |
| Deciding what to write, guards | `crates/core/src/engine.rs` |
| Per-side hashes and writer revisions | `crates/core/src/ledger.rs` |
| Atomic writes and backups | `crates/core/src/fsx.rs` |
| Anything that knows an agent's file format | `crates/core/src/adapters/` |
| Reading roots, flags, environment, the lock, exit codes, JSON output | `crates/cli/src/main.rs` |
| The MCP hub: `hub.toml`, secrets, proxying, import, OAuth sign-in, registration in the agents, subagents | `crates/hub/`, `crates/cli/src/mcp.rs`, `crates/cli/src/subagent.rs`, `wiki/architecture/hub.md`, `wiki/decisions/mcp-hub.md` |
| Compatibility probes, status, state file, prompt | `crates/core/src/compat.rs`, `crates/core/src/adapters/*_probes.rs`, `crates/cli/src/doctor.rs` |
| Fixture homes for tests | `crates/core/src/testkit.rs` |
| Releases, release notes, macOS packaging | `scripts/release.sh`, `apps/macos/scripts/bundle.sh`, `wiki/guides/release.md` |
| Continuous integration | `.github/workflows/`, `wiki/architecture/ci.md` |
| Demo data | `scripts/demo-home.sh` |
| Display names of agents in the app | `apps/macos/Sources/NativememKit/MemoryIndex.swift` (`agentDisplayName`) |

## Source anchors

- `Cargo.toml`
- `crates/core/src/lib.rs`
- `crates/core/src/adapters/mod.rs`
- `crates/hub/src/lib.rs`
- `crates/cli/src/doctor.rs`
- `apps/macos/Package.swift`
- `scripts/release.sh`
- `.github/workflows/ci.yml`
- `.gitignore`

## Rules

- MUST: Format knowledge for an agent lives under `crates/core/src/adapters/`, never in `engine.rs`.
- MUST: Defaults for agent roots are resolved only in `crates/cli/src/main.rs`.
- MUST: A new file or folder is added to the tree above in the same task.
- SHOULD: Fixtures for a new agent go into `crates/core/src/testkit.rs` as a home builder.

## Verification checklist

- [ ] Every path in the tree exists (`ls`)
- [ ] The module list in `crates/core/src/lib.rs` and `crates/core/src/adapters/mod.rs` matches the tree
- [ ] `wiki/README.md` index matches the `wiki/` subtree

## Related docs

- `wiki/architecture/overview.md`
- `wiki/architecture/hub.md`
- `wiki/WIKI-CREATOR.md`
- `wiki/guides/add-an-agent.md`

## Last updated

2026-11-11
