# Wiki writing rules (for agents)

## Purpose

Where to write what, and how. Main rule: **context is everything.** Structure exists only to make
context reachable.

## Scope

Every page under `wiki/`. The wiki records the current state; history lives in git.

## When to write wiki

- A new agent, resource kind, policy or decision exists
- A format an agent reads or writes changes, or a new fact about an agent is verified
- The CLI flags, environment variables, exit codes or JSON contract change
- The desktop app gains or loses a behaviour that the CLI contract or `docs/desktop.md` describes
- A safety rule changes (guard thresholds, backup layout, locking, test isolation)
- A directory, crate or module is added, removed or changes ownership
- A rollout against a real agent teaches something (a quirk, a check that caught a bug)

## Where to write (routing)

| What changed | Where |
|---|---|
| New agent | `wiki/agents/<agent>.md` from `wiki/_templates/AGENT.md` (with its probe table); `wiki/architecture/compatibility.md` check list; index row in `wiki/README.md`; support table in `README.md`; `wiki/architecture/file-structure.md` (new adapter file); `wiki/architecture/overview.md` capability table; `wiki/guides/verify-with-real-agents.md` checks for the agent |
| New resource kind | `wiki/kinds/<kind>.md`; `wiki/architecture/overview.md` (policy per kind, `supports` table); `wiki/guides/add-a-resource-kind.md` if the steps changed; `wiki/decisions/` for the policy choice; the kind row in every `wiki/agents/*.md` status section |
| Changed agent format (path, schema, marker, encoding) | The agent's page (Native locations, Formats, Facts table with evidence level, the probe table with its expected constants) and the kind page that describes the marker; `Last updated` on both |
| New or changed compatibility probe, status rule, state file, prompt or notification | `wiki/architecture/compatibility.md`; the agent's probe table in `wiki/agents/<agent>.md`; `wiki/runbooks/agent-update-action-required.md` if the steps changed; `docs/desktop.md` for the `doctor --json` contract (report drift, do not edit it unasked) |
| Changed CLI flag, environment variable, exit code or JSON contract | `README.md` usage; `wiki/architecture/overview.md` (CLI section); `docs/desktop.md` contract is a spec outside the wiki: report drift, do not edit it unasked; `wiki/guides/add-an-agent.md` if the root flags changed |
| Desktop app change | `wiki/architecture/file-structure.md` (app files); `wiki/architecture/overview.md` (app section); `wiki/architecture/safety.md` if the app gains a write path |
| Safety rule change | `wiki/architecture/safety.md` first, then `wiki/CLAUDE.md` invariants and the root `CLAUDE.md` hard rules |
| New decision | `wiki/decisions/<decision>.md` with current-state wording ("X does Y because Z"); index row in `wiki/README.md`; link from the pages it governs |
| Verified or disproved agent fact | The agent's Facts table: change the evidence level, never leave a stale tag |
| New directory or module | `wiki/architecture/file-structure.md` |
| Release process, version rules, signing, artifact names | `wiki/guides/release.md`; `scripts/release.sh` and `apps/macos/scripts/bundle.sh` are the source; `README.md` install section |
| CI workflow, runner or check change | `wiki/architecture/ci.md`; the commands in `CLAUDE.md` and `wiki/CLAUDE.md` when they change |
| Packaging (app bundle, universal build, signing identity) | `wiki/guides/release.md`; `wiki/architecture/file-structure.md` |
| Demo data (`scripts/demo-home.sh`) | `wiki/architecture/file-structure.md`; `CONTRIBUTING.md` if the workflow for contributors changes |
| New page | Index row in `wiki/README.md`, a `Related docs` link from at least one existing page |

An existing topic is updated in place: no change log, `Last updated` moves forward.

## Mandatory sections (new pages)

Purpose · Scope · Source anchors · Rules (MUST / SHOULD) · Verification checklist · Related docs ·
Last updated (`YYYY-MM-DD`). Skeletons: `wiki/_templates/PAGE.md`, `wiki/_templates/AGENT.md`.
Content sections (tables, formats, decision text) sit between Scope and Source anchors.

## Style

- Short sentences; no vague advice.
- Every number or claim names its source: a repo path, a test name, or an evidence level for agent
  facts. Unverified claims carry the literal tag `[UNVERIFIED]`.
- Current-state wording only: say what X does and why, never what it was, what it replaced or what
  is retired. Words that narrate history are allowed only as a literal field or value quoted from
  a file.
- Placeholder paths only: `/Users/x/...`, projects named `app` or `acme`. No real user names, real
  project names or file contents from a real machine.
- No placeholder or empty pages.

## Source anchors

- `wiki/_templates/PAGE.md`
- `wiki/_templates/AGENT.md`
- `wiki/architecture/context-quality.md`
- `wiki/architecture/file-structure.md`

## Rules

- MUST: Route every change through the table above before finishing a task.
- MUST: Add a new page to `wiki/README.md` in the same edit.
- MUST: Keep the mandatory sections on every page.
- SHOULD: Prefer editing the page that owns a topic over adding a new page.

## Verification checklist

- [ ] The routing table row for the change was followed
- [ ] `Last updated` moved on every touched page
- [ ] A grep for the forbidden history words and for personal data in `wiki/` returns nothing

## Related docs

- `wiki/README.md`
- `wiki/CLAUDE.md`
- `wiki/architecture/context-quality.md`

## Last updated

2026-11-11
