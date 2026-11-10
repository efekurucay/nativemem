# Pi

## Purpose

What `nativemem` needs to know about the Pi coding agent to sync instruction sections, project
memory and sessions with it.

## Scope

Facts gathered read-only on a development machine running Pi 1.5.0 (session file format version 3;
380 session files, 60,121 entries) from the installed package sources and local session files. The
`pi` binary is never driven by this repo.

## Status in nativemem

| Kind | Status | Adapter |
|---|---|---|
| Project memory | Replicas only (Pi has no native memory): store plus routing block | `crates/core/src/adapters/agents_file.rs`, `memory_store.rs` |
| Instruction sections | Read and write in `AGENTS.md` | `agents_file.rs` through `sections.rs` |
| Sessions | Read (originals) and write (replicas) | `crates/core/src/adapters/pi.rs` |
| MCP servers, skills | Not implemented | |

## Native locations

| What | Global | Project |
|---|---|---|
| Instructions | `<pi home>/AGENTS.md` (the loader tries `AGENTS.override.md`, `AGENTS.md`, `AGENTS.MD`, `CLAUDE.md`, `CLAUDE.MD` in that order and takes the first) | The same candidate list in every directory from the cwd up to the filesystem root, loaded root first after the global file |
| Memory | none native; replica store `<pi home>/nativemem/memory/<project key>/` | |
| Sessions | `<pi home>/sessions/--<cwd>--/<ISO ts>_<session id>.jsonl` | |
| MCP servers | `<pi home>/mcp.json` key `mcpServers` | `.pi/mcp.json` (only after project trust) |
| Skills | `<pi home>/skills/`, `~/.agents/skills/`, paths listed in `settings.json` `skills[]` | `.pi/skills/`, `.agents/skills/` (only after project trust) |

The Pi home is `--pi-home`, else `NATIVEMEM_PI_HOME`, else `PI_CODING_AGENT_DIR`, else `~/.pi/agent`.

## Formats

- **Session file**: JSONL, header first: `{"type":"session","version":3,"id","timestamp","cwd"}`; every
  other entry has `type`, `id` (8 lowercase hex), `parentId` (string or null) and an ISO `timestamp`.
  Entry types: `message`, `model_change`, `thinking_level_change`, `compaction`, `session_info`,
  `custom`, and others defined in code. Message roles: `user`, `assistant` (content blocks `text`,
  `thinking`, `toolCall`), `toolResult`, `bashExecution`. Assistant messages carry `api`, `provider`,
  `model`, `usage`, `stopReason`. The first entry after the header has `parentId` null and each
  appended entry points at the previous leaf; context is the walk from the leaf to the root.
- **Directory name**: `--` + working directory with one leading separator dropped and every `/`, `\`
  and `:` replaced by `-` + `--` (`encode_cwd` in `crates/core/src/adapters/pi.rs`).
- **Instruction file and memory**: markdown; see `wiki/kinds/instruction-sections.md` and
  `wiki/kinds/memory.md`.
- **MCP entry** (not implemented): `command`, `args`, `env`, `cwd`, `url`, `headers`, `oauth`,
  `enabled`, `exposure`; values accept `${VAR}` and `!command` forms.

## What nativemem reads and writes today

- Reads `AGENTS.md` sections and `sessions/**/*.jsonl`.
- Writes instruction replicas in the region at the end of `AGENTS.md`, the replica memory store under
  `nativemem/memory/`, the routing block in `AGENTS.md`, and session replicas.
- A session replica has the header, a `custom` marker entry (`customType` `nativemem-origin`,
  `data.origin`), a `session_info` entry and one linear `message` entry per turn. The file name is
  `<creation ISO ts>_<uuid derived from the origin>.jsonl`; a refresh rewrites the same file. The
  file mtime is the original's.
- The conversation read from a Pi file is the active branch: the path from the last entry to the
  root.

## Sessions: listing and resume

- `pi --resume` and `/resume` open a picker built from the session directory of the current cwd, with
  an "all" scope that scans every directory under `sessions/`. Each entry shows the last
  `session_info` name, else the first user message. The sort key is the newest user or assistant
  message timestamp inside the file (OBSERVED in the package source).
- `pi --continue` takes the most recently modified `.jsonl` (file mtime) in the cwd's directory
  whose header parses. A replica is chosen by `--continue` only when its mtime is the newest, and it
  gets the original's mtime.
- Files are recognised by the `.jsonl` suffix and a valid first-line header. There is no index or
  database; adding or removing files is enough.
- A written replica needs: the header with the real `cwd`, entries linearly chained by `parentId`,
  and at least one message.

## Quirks

- **Pi restores the model from the session and warns on unknown models.** It reads the model from the
  last `model_change` entry or the last assistant message that is not virtual. A replica has no
  `model_change` entry and its assistant messages carry `api: "pi-virtual"`, which Pi skips, so the
  replica opens with Pi's own default model without a warning (test `replica_names_no_restorable_model_and_has_no_setup_entries`).
- **The directory name is lossy**: `/a-b` and `/a/b` collide. The header `cwd` is the only trusted
  path (380 of 380 local files had directory name equal to the encoding of the header cwd).
- **Instruction candidates are ordered.** If `AGENTS.override.md` exists in the Pi home it is loaded
  instead of `AGENTS.md`, so synced sections would not reach the model (OBSERVED in the package
  source; the README states the same).
- **No size cap on instruction files**: content is inlined whole, so the routing block costs tokens
  in every session.
- **No approval layer for file reads.** The `read` tool resolves absolute paths and `~` without
  asking, so the routing block works; output is capped at 2000 lines or 50 KB per read.
- **Provider replay blobs are opaque**: `thinkingSignature` and similar are bound to a provider;
  thinking blocks are dropped on conversion.
- **Native MCP exists since 1.5.0** (stdio and streamable HTTP; SSE rejected). An installed extension
  that registers `/mcp` replaces the built-in support and Pi stops reading `mcp.json`.
- **Reload**: `/reload` after edits made outside the session.

## Version detection and compatibility probes

The version is the `version` of the `package.json` found above the resolved `pi` executable: the
first `pi` on `PATH` is located and its symlinks are resolved, never run (`version` in
`crates/core/src/adapters/pi_probes.rs`); without one the version is unknown. A scheduled job has a
minimal `PATH`, so it often cannot see the version; the remembered one stays. Session replicas are
written with header `version: 3`.

The probes (`wiki/architecture/compatibility.md`) read the newest session for the header and the
newest 3 sessions (last 1 MiB each) for entry types.

| Check id | Protects | Expected constant | Fails when |
|---|---|---|---|
| `pi.session_header` | session | `HEADER_TYPE` `session`, `HEADER_VERSION` `3`, keys `cwd`, `id`, `timestamp` | the header has another `type` or `version`, or lacks a key |
| `pi.entry_types` | session | `KNOWN_ENTRY_TYPES`: `message`, `session_info`, `custom`, `custom_message`, `model_change`, `thinking_level_change`, `compaction`, `branch_summary`, `label` | an entry has another `type` |
| `pi.cwd_encoding` | session | the directory name equals `encode_cwd(header cwd)` | a sampled session (newest 8) sits in another directory |

Evidence (read-only statistics on the development machine, Pi 1.5.0): the headers of the newest 30
sessions all read `type: session`, `version: 3` with the keys above; those sessions held 6 entry
types, and the package source (`dist/core/session-manager.js`) defines the other 3; 60 of 60 sampled
directory names equal the encoding of the header `cwd`.

## Facts and evidence

| Fact | Evidence level | Source |
|---|---|---|
| Directory name equals the encoding of the header cwd | VERIFIED (380 of 380 files) | local corpus; test `encodes_cwd_like_pi` |
| 377 of 380 files are strictly linear; 3 have branches; 1 has two roots | OBSERVED | local corpus |
| The picker shows `session_info` name else first user message | OBSERVED | package source |
| `--continue` sorts by file mtime | OBSERVED | package source |
| A replica opens without a model warning | INFERRED from Pi's model-restore code | `REPLICA_API` comment in `pi.rs`; the staged check confirms it |
| Pi lists a nativemem-written session in its picker | `[UNVERIFIED]` until the staged check is recorded | `wiki/guides/verify-with-real-agents.md` |
| The three probes pass on the fixtures; header version 4, an unknown entry type and a moved session directory each fail one check | VERIFIED | `crates/core/tests/probes.rs` (`pi_*`) |
| An `AGENTS.override.md` shadows `AGENTS.md` | OBSERVED | package source |
| Global context candidates and ancestor loading | OBSERVED | package source |

## Source anchors

- `crates/core/src/adapters/pi.rs`
- `crates/core/src/adapters/pi_probes.rs`
- `crates/core/src/adapters/agents_file.rs`
- `crates/core/src/adapters/memory_store.rs`
- `crates/core/src/testkit.rs` (`PiHome`)
- `crates/core/tests/probes.rs`
- `README.md` (Notes)

## Rules

- MUST: Keep `api: "pi-virtual"` on replica assistant messages and no `model_change` entry.
- MUST: Mark replicas with the `nativemem-origin` custom entry.
- MUST: Set the replica file mtime to the original's.
- MUST: Update the probe constants in `pi_probes.rs` and the table above together with the adapter.
- SHOULD: Check for `AGENTS.override.md` when an instruction section does not reach Pi.

## Verification checklist

- [ ] The staged check for Pi (picker lists the copy, opening it shows no model warning) is recorded here when run
- [ ] `SESSION_WRITER_REVISION` in `pi.rs` matches `wiki/architecture/overview.md`
- [ ] The README support table row for Pi matches this page

## Related docs

- `wiki/architecture/compatibility.md`
- `wiki/runbooks/agent-update-action-required.md`
- `wiki/kinds/sessions.md`
- `wiki/kinds/memory.md`
- `wiki/guides/verify-with-real-agents.md`
- `wiki/agents/opencode.md`

## Last updated

2026-11-11
