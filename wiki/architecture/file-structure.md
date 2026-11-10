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
├── Cargo.toml                    workspace: crates/core, crates/cli; version and package metadata
├── LICENSE                       MIT
├── CONTRIBUTING.md · SECURITY.md contribution rules; private vulnerability reporting
├── CLAUDE.md                     entry point for coding agents (AGENTS.md is a symlink to it)
├── README.md                     user-facing overview, usage, support table
├── docs/
│   ├── design.md                 original design and build plan
│   ├── desktop.md                desktop app spec and the CLI JSON contract
│   └── images/                   screenshots referenced by README.md (overview.png, memory-detail.png)
├── scripts/
│   ├── release.sh                local release: checks, version bump, macOS artifacts, notes, commit, tag
│   └── demo-home.sh              synthetic agent homes with generic memory items for trying the CLI and app
├── .github/
│   ├── workflows/                ci.yml (Linux checks), release.yml (Linux binaries, release), portability.yml (manual Windows build)
│   ├── ISSUE_TEMPLATE/           bug_report, agent_update, config
│   ├── PULL_REQUEST_TEMPLATE.md
│   └── dependabot.yml            weekly grouped cargo and actions updates
├── crates/
│   ├── core/                     library `nativemem-core`: no agent format knowledge in the engine
│   │   └── src/
│   │       ├── lib.rs            module list
│   │       ├── model.rs          AgentId, Scope, Kind, Item, ItemKey, Observed, the Adapter trait, content_hash
│   │       ├── engine.rs         plan (pure) and apply: three-way decision, session planner, mass-delete guard
│   │       ├── ledger.rs         SQLite ledger: items and replicas tables, writer revision column
│   │       ├── fsx.rs            atomic_write, BackupSession, backup layout
│   │       ├── session.rs        neutral session model, tool rendering, titles, window, markers, derived ids
│   │       ├── compat.rs         Check, AgentReport, Status, compat.json state, assess, prompt, notifications
│   │       ├── probe.rs          read-only helpers for probes: tails, newest files, executable lookup
│   │       ├── slug.rs           slugify
│   │       ├── error.rs          Error enum (Io, Parse, Ledger, Unsupported, MassDelete)
│   │       ├── testkit.rs        fixture builders: ClaudeHome, CodexHome, PiHome (temp dirs only)
│   │       └── adapters/
│   │           ├── mod.rs        adapter module list
│   │           ├── claude.rs     Claude Code: project memory, CLAUDE.md sections, session replicas
│   │           ├── claude_sessions.rs  Claude transcript reader and writer
│   │           ├── codex.rs      Codex: generated-memory originals, replica store, AGENTS.md, rollouts
│   │           ├── codex_sessions.rs   Codex rollout reader (originals, replica detection, automated markers)
│   │           ├── codex_threads.rs    `threads` row writer for replica rollouts, database backup and retention
│   │           ├── codex_rollout.rs    Codex rollout writer shaped like Codex's own importer output
│   │           ├── codex_imports.rs    Codex importer ledger (`external_agent_session_imports.json`)
│   │           ├── claude_probes.rs    Claude compatibility probes and their expected constants
│   │           ├── codex_probes.rs     Codex probes: state database, rollouts, import ledger
│   │           ├── pi_probes.rs        Pi probes: session header, entry types, directory encoding
│   │           ├── opencode_probes.rs  OpenCode probes: config directory and AGENTS.md
│   │           ├── pi.rs         Pi: AGENTS.md through agents_file, session reader and writer
│   │           ├── agents_file.rs      generic adapter for agents that only read a global AGENTS.md (OpenCode, Pi)
│   │           ├── memory_store.rs     replica store plus routing block, shared by Codex, OpenCode, Pi
│   │           └── sections.rs   instruction-section parser, managed regions, byte-preserving edits, markers
│   │   └── tests/probes.rs       probes against testkit fixtures, drift cases, blocked kinds
│   └── cli/                      binary `nativemem`
│       ├── src/
│       │   ├── main.rs           clap CLI, root resolution, lock, status/items/sync/delete/edit, JSON output
│       │   ├── doctor.rs         `doctor`, compatibility reports, warnings, notification delivery
│       │   └── schedule.rs       launchd scheduling (macOS only)
│       └── tests/                cli.rs (end-to-end CLI tests over temp homes), doctor.rs (doctor, gating, notifications)
├── apps/macos/                   Swift package, macOS 14+, no third-party dependencies
│   ├── Package.swift
│   ├── Sources/Nativemem/        app entry point (`NativememApp.swift`)
│   ├── Sources/NativememKit/     app logic and views
│   │   ├── NativememCLI.swift    finds and runs the binary, decodes JSON
│   │   ├── Contract.swift        Codable types mirroring the CLI JSON contract
│   │   ├── AppModel.swift        observable state, grouping, copy folding
│   │   ├── MemoryIndex.swift     agent display names, project keys, grouping of items
│   │   ├── ProjectTree.swift     folder tree built from project paths
│   │   ├── SyncSession.swift     preview then apply flow
│   │   ├── BodyText.swift, MarkdownBlocks.swift  body rendering
│   │   └── SidebarView, ItemListView, ItemDetailView, OverviewView, SyncSheet, ContentView
│   ├── Tests/NativememKitTests/  contract, doctor, index, tree, sectioning, body parser, sync headline tests
│   │   └── Fixtures/             one JSON file per CLI document shape
│   ├── Resources/AppIcon.png
│   └── scripts/bundle.sh         builds the app bundle into `apps/macos/build/` (`VERSION`, `UNIVERSAL`, `NATIVEMEM_CLI`, `CODESIGN_IDENTITY`)
└── wiki/                         this knowledge base
    ├── README.md · CLAUDE.md · WIKI-CREATOR.md
    ├── architecture/             ci, compatibility, context-quality, file-structure, overview, safety
    ├── kinds/                    memory, instruction-sections, sessions
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
- `wiki/WIKI-CREATOR.md`
- `wiki/guides/add-an-agent.md`

## Last updated

2026-11-11
