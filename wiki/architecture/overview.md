# Architecture overview

## Purpose

How `nativemem` keeps agents in sync: adapters, the neutral model, the ledger, the decision table,
capability negotiation, writer revisions and the policy per resource kind.

## Scope

The sync core and the CLI. Per-kind detail is in `wiki/kinds/`, per-agent detail in `wiki/agents/`,
write safety in `wiki/architecture/safety.md`.

## Pieces

| Piece | Role | Anchor |
|---|---|---|
| Adapter | One per agent (OpenCode and Pi share a generic one for `AGENTS.md`). Scans the agent's native store and writes, updates and removes entries in the agent's own format | `crates/core/src/model.rs` (`Adapter`), `crates/core/src/adapters/` |
| Neutral model | `Item { kind, scope, name, description, category, body }` and `Observed` (an item found on disk, with `replica_of` when it carries a marker) | `crates/core/src/model.rs` |
| Ledger | SQLite file `<state dir>/ledger.db` | `crates/core/src/ledger.rs` |
| Compatibility watch | Probes per adapter, `compat.json`, status and prompt; pauses the kinds of failing checks | `crates/core/src/compat.rs`, `wiki/architecture/compatibility.md` |
| Engine | `plan` is pure; `apply` executes and updates the ledger. It knows nothing about formats | `crates/core/src/engine.rs` |
| CLI | Resolves roots, takes the lock, prints text or one JSON document | `crates/cli/src/main.rs` |
| Desktop app | A window onto the CLI; holds no sync logic | `docs/desktop.md`, `apps/macos/Sources/NativememKit/NativememCLI.swift` |

An agent takes part in a run only when its home directory exists (`Agents::open` in
`crates/cli/src/main.rs`; test `agent_without_a_home_directory_takes_no_part`).

## Neutral model

- `AgentId` is the agent name: `claude`, `codex`, `opencode`, `pi`.
- `Scope` is `Global` or `Project(<absolute working directory>)`.
- `Kind` is `Memory`, `Instruction` or `Session` (`as_str`: `memory`, `instruction`, `session`).
- `ItemKey { origin, native_id }` identifies an original. `native_id` is stable inside one agent.
- `content_hash(item)` is SHA-256 over `name`, `description`, `category` and `body`, each
  length-prefixed, after CRLF becomes LF and trailing whitespace is trimmed. Scope is identity, not
  content, so it is not hashed. A session hashes its content only (working directory, title, each
  turn's role and text), so a replica hashes like its original (tests `scope_is_not_hashed`,
  `hash_covers_content_but_not_ids_or_times` in `crates/core/src/model.rs` and
  `crates/core/src/session.rs`).

## Ledger with per-side hashes

| Table | Columns | Meaning |
|---|---|---|
| `items` | `origin`, `native_id`, `kind`, `scope`, `hash` | The original's content hash at the last successful sync |
| `replicas` | `origin`, `native_id`, `agent`, `replica_native_id`, `path`, `hash`, `revision` | One row per replica: where it is, the replica's own hash at the last sync, and the writer revision it was written with |

Per-side hashes are what make a three-way decision possible: the engine compares the original's
current hash with `items.hash`, and each replica's current hash with its own `replicas.hash`. The
ledger stores ids, hashes and paths, never entry text. An empty `replicas.hash` freezes a session
replica (a tombstone, or a replica another program wrote). The `revision` column is added on open
when it is missing, and rows without it read as revision 0 (test
`ledger_without_revision_column_migrates_and_reads_revision_zero`).

## Three-way decision table (memory and instruction items)

Implemented in `plan_key` in `crates/core/src/engine.rs`. "Changed" means the current hash differs
from the ledger hash of that side.

| Original | Replica in agent B | Action | Test |
|---|---|---|---|
| new, not in ledger | none | write a replica to every other agent that supports the item | `new_claude_memory_reaches_codex` |
| changed | unchanged | update replicas | `original_edit_updates_replica` |
| unchanged | changed | write the edit back to the original, then to the other replicas | `replica_edit_updates_original_including_category` |
| changed | changed (or two replicas changed) | conflict note, nothing written | `both_sides_edited_is_a_conflict` |
| unchanged | file missing, ledger row present | remove the original and the other replicas, forget the item | `deleting_replica_removes_original_with_backup` |
| changed, or another replica changed | file missing | conflict note "replica deleted, other side edited" | `crates/core/src/engine.rs` plan code |
| missing, ledger row present | unchanged | remove replicas, forget the item | `deleting_original_removes_replica_and_routing` |
| missing | changed | conflict note "original missing, replica edited" | `crates/core/src/engine.rs` plan code |
| any | marked, no ledger row | adopt: record it, write nothing | `lost_ledger_is_rebuilt_by_adoption` |
| not found | marked, no ledger row | skipped note "orphan replica", nothing removed | `orphan_replica_is_reported_not_deleted` |
| Codex-generated original | edited anywhere | write-back is unsupported: skipped note, both sides left alone | `codex_original_replicates_to_claude_and_cannot_be_written_back` |

Apply order is adopt and tombstone, then writes, then removals, then forgets (`Action::rank`). After
a run, `refresh` rescans and records what is actually on disk for every touched item, so formats
that do not round-trip byte-identically never look like fresh edits. A failed scan aborts the run
before planning (test `failed_scan_plans_nothing`).

`Adapter::finish` runs once at the end of `apply`, after the writes and the refresh. Codex uses it to write
the `threads` rows of its replica rollouts; what an adapter had to leave for the next run comes back as a
`skipped` note (`finish_all` in `crates/core/src/engine.rs`).

Sessions use a different planner (`plan_session`), described in `wiki/kinds/sessions.md`.

## Capability negotiation: `supports`

`Adapter::supports(kind, scope)` says what an agent can store. The engine leaves agents that do not
support an item out of its replica targets, write-back targets, absent sets and adoption, so they
produce no actions, skipped notes or counts for it. `Error::Unsupported` stays as a runtime
fallback. Test: `memory_is_planned_only_for_agents_that_support_it`.

| Agent | Memory (Project) | Instruction (Global) | Session (Project) |
|---|---|---|---|
| Claude Code | yes, originals and replicas | yes | yes |
| Codex | yes; originals are generated and read-only, replicas are written | yes | only with `NATIVEMEM_CODEX_SESSION_WRITES=on` while the Codex schema probes pass; each written replica also gets a `threads` row |
| OpenCode | replicas only | yes | no |
| Pi | replicas only | yes | yes |

`Adapter::accepts(item)` (default true) is a second gate for one item: the engine plans nothing for a
session a target does not accept, as Claude Code does for sessions older than its retention. Every
adapter also has `with_blocked_kinds(...)`: a kind in that list makes `supports` false. The CLI
fills it from failing compatibility checks, so a paused kind is neither read from nor written to the
agent and what exists for it is left alone (`wiki/architecture/compatibility.md`).

Anchors: `supports` in `crates/core/src/adapters/claude.rs`, `codex.rs`, `pi.rs`, `agents_file.rs`.

## Writer revisions

`Adapter::writer_revision(kind)` is the version of the adapter's output format for a kind (0 unless
overridden). The ledger records the revision a replica was written with. A session replica with a
lower revision is rewritten (the file is backed up first) while it is unmodified; a modified one is
left alone. Adopted replicas are recorded at the current revision. Current values: Claude sessions
1, Codex sessions 1, Pi sessions 2, everything else 0
(`SESSION_WRITER_REVISION` in the three adapter files). Bump the constant whenever the written
session format changes. Tests: `unmodified_replica_with_an_older_writer_revision_is_rewritten_once`,
`modified_replica_with_an_older_writer_revision_is_left_alone`.

## Policy per kind

| Kind | Policy | Deletes | Edits on a replica | Page |
|---|---|---|---|---|
| Memory | Faithful replication in both directions | Propagate, with backup | Written back to the original | `wiki/kinds/memory.md` |
| Instruction | Faithful replication, one `## ` section per item | Propagate, with backup | Body written back; the heading is identity | `wiki/kinds/instruction-sections.md` |
| Session | Snapshot: copy once, refresh while untouched | Never propagate | Never flow back | `wiki/kinds/sessions.md` |

## CLI

Commands: `status`, `sync` (dry run unless `--apply`; `--notify` with `--apply`), `doctor`, `items`,
`delete`, `edit`, `schedule install|uninstall|status` (macOS). Root flags and variables: `--claude-home`, `--codex-home`,
`--opencode-home`, `--pi-home`, `--state-dir` (`NATIVEMEM_CLAUDE_HOME`, `NATIVEMEM_CODEX_HOME`,
`NATIVEMEM_OPENCODE_HOME`, `NATIVEMEM_PI_HOME`, `NATIVEMEM_STATE_DIR`), `--session-days`
(`NATIVEMEM_SESSION_DAYS`), `--exclude-automated-sessions`, `--no-sessions`. Without a flag, a root
falls back to `CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `OPENCODE_CONFIG_DIR`, `PI_CODING_AGENT_DIR`, then
`~/.claude`, `~/.codex`, `~/.config/opencode`, `~/.pi/agent`; the state directory defaults to
`~/.nativemem`. `NATIVEMEM_CODEX_SESSION_WRITES=on` enables Codex session writes (off by default). `NATIVEMEM_REPO` names the repo in
`doctor` prompts; `NATIVEMEM_NOTIFY_FILE` redirects notifications to a file. Exit codes: 0
success, 2 when conflicts or skipped writes remain, 3 when `doctor` finds an agent that needs action,
1 on error. With `--json`, one JSON document is
printed, and errors are `{"error": ..., "code": ...}` with codes `busy`, `mass_delete`,
`unsupported`, `invalid`, `not_found`, `unknown_agent`, `not_installed`, `io`, `parse`, `ledger`,
`unsupported_platform`. The JSON shapes are specified in `docs/desktop.md`. Anchor: `Failure::code`
in `crates/cli/src/main.rs`.

## Desktop app

`apps/macos` runs CLI commands with `--json` and renders the result. Display names come from
`agentDisplayName` in `apps/macos/Sources/NativememKit/MemoryIndex.swift`. Chat sessions are
filtered out of the item views (`all.filter { $0.kind != Item.sessionKind }` in the same file);
`status` counts them separately. The app finds the binary through app settings, `NATIVEMEM_BIN`, a
file next to the executable, then `~/.cargo/bin`, `/usr/local/bin`, `/opt/homebrew/bin`
(`apps/macos/Sources/NativememKit/NativememCLI.swift`). It also runs `doctor --json` and shows the
compatibility notices on the Overview (`wiki/architecture/compatibility.md`).

## Source anchors

- `crates/core/src/model.rs`
- `crates/core/src/engine.rs`
- `crates/core/src/ledger.rs`
- `crates/cli/src/main.rs`
- `crates/cli/src/doctor.rs`
- `crates/cli/tests/cli.rs`
- `docs/design.md`
- `docs/desktop.md`

## Rules

- MUST: The engine stays format-agnostic; add formats as adapters.
- MUST: Declare what an adapter can store in `supports`; do not rely on `Error::Unsupported`.
- MUST: Bump `SESSION_WRITER_REVISION` of an adapter whenever its written session format changes.
- MUST: Keep the decision table in this page equal to `plan_key`.
- SHOULD: Express a new behaviour as a policy per kind before adding engine branches.

## Verification checklist

- [ ] Each table row names a test or the code that implements it
- [ ] The capability table matches the `supports` implementations
- [ ] The writer revision values match the constants in the adapter files

## Related docs

- `wiki/architecture/safety.md`
- `wiki/architecture/compatibility.md`
- `wiki/kinds/memory.md`
- `wiki/kinds/instruction-sections.md`
- `wiki/kinds/sessions.md`
- `wiki/guides/add-an-agent.md`

## Last updated

2026-11-11
