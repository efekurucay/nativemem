# nativemem wiki

## Purpose

Index and reading order for the knowledge base of `nativemem`, a deterministic CLI that keeps each
coding agent's own memory, global instruction sections and chat sessions in sync with the other
agents, without a shared store. Context is everything: every page exists so an engineer or a coding
agent can act on this repo without re-deriving facts.

## Scope

| In scope | Out of scope |
|---|---|
| `wiki/**`, the root `CLAUDE.md` and `AGENTS.md` | Code comments, `docs/design.md` and `docs/desktop.md` (they stay the original design and the app spec; the wiki records the current behaviour and points at them) |

## Reading order

1. `wiki/CLAUDE.md` for the rules of working here
2. `wiki/architecture/overview.md` for how the pieces fit
3. `wiki/architecture/safety.md` before any write path or test is touched
4. The page of the kind or agent the task touches
5. `wiki/WIKI-CREATOR.md` before finishing, to route what changed to the page that must be updated

## Index

| Page | What it answers |
|---|---|
| `wiki/CLAUDE.md` | Rules and invariants for agents working in this repo |
| `wiki/WIKI-CREATOR.md` | When to write wiki, and the routing table "what changed, where to write it" |
| `wiki/architecture/context-quality.md` | The quality bar every page must meet, evidence levels |
| `wiki/architecture/file-structure.md` | The real tree and what each crate, module and folder owns |
| `wiki/architecture/overview.md` | Adapters, neutral model, ledger, three-way decision table, `supports`, writer revisions, policy per kind |
| `wiki/architecture/ci.md` | GitHub Actions workflows, runners, what is best-effort and what runs only locally |
| `wiki/architecture/compatibility.md` | Compatibility watch: probes, status, `compat.json`, gating, `doctor`, prompt, notifications |
| `wiki/architecture/safety.md` | Backups, atomic writes, mass-delete guard, lock, no writes in git repos, secrets, test isolation, staged rollout |
| `wiki/kinds/memory.md` | Project memory entries: model, stores, markers, write-back |
| `wiki/kinds/instruction-sections.md` | Global instruction files as one item per `## ` section |
| `wiki/kinds/sessions.md` | Chat sessions: neutral model, snapshot policy, markers, tool rendering, window flags |
| `wiki/agents/claude-code.md` | Claude Code locations, formats, quirks |
| `wiki/agents/codex.md` | Codex locations, formats, rollouts, thread rows, quirks |
| `wiki/agents/pi.md` | Pi locations, formats, model restore, quirks |
| `wiki/agents/opencode.md` | OpenCode locations, formats, fallback to the Claude file, quirks |
| `wiki/agents/hermes.md` | Hermes verified facts and recommended native placement; no adapter exists |
| `wiki/runbooks/agent-update-action-required.md` | What to do when an agent shows ActionRequired: prompt flow, `--accept`, rollback |
| `wiki/guides/add-an-agent.md` | End-to-end checklist for a new agent |
| `wiki/guides/add-a-resource-kind.md` | End-to-end checklist for a new resource kind |
| `wiki/guides/release.md` | Cutting a release with `scripts/release.sh`: notes, artifacts, versioning, signing, rollback |
| `wiki/guides/verify-with-real-agents.md` | Staged rollout recipe and rollback from `~/.nativemem/backups` |
| `wiki/decisions/no-shared-store.md` | Decision: there is no shared store |
| `wiki/decisions/native-flow-only.md` | Decision: only files and formats the agents already read |
| `wiki/decisions/session-snapshot-policy.md` | Decision: sessions are snapshots, deletes never propagate |
| `wiki/decisions/instruction-sections-as-items.md` | Decision: one `## ` section is one item |
| `wiki/decisions/routing-block-for-agents-without-memory.md` | Decision: a marked block in the global instruction file points at per-project indexes |
| `wiki/decisions/codex-session-writes-gated.md` | Decision: Codex session writes are guarded by the schema probes and always carry a thread row |
| `wiki/_templates/PAGE.md` | Skeleton for a new page |
| `wiki/_templates/AGENT.md` | Skeleton for a new agent page |

## Source anchors

- `README.md`
- `docs/design.md`
- `docs/desktop.md`
- `crates/core/src/lib.rs`
- `crates/cli/src/main.rs`
- `scripts/release.sh`

## Rules

- MUST: Every wiki page is listed in the index above.
- MUST: Update the wiki in the same task as the code change that made it wrong.
- SHOULD: Read only the pages the task needs; the reading order lists the minimum.

## Verification checklist

- [ ] Every page under `wiki/` appears in the index
- [ ] Every path in backticks that points into the repo exists
- [ ] No page contains a placeholder or a change log

## Related docs

- `wiki/CLAUDE.md`
- `wiki/WIKI-CREATOR.md`

## Last updated

2026-11-11
