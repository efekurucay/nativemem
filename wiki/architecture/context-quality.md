# Context quality

## Purpose

The bar every wiki page must meet so the next engineer or coding agent can act on it without
re-deriving facts.

## Scope

All pages under `wiki/`. Agent pages carry the most unverified material, so the evidence levels
below exist mainly for them.

## Evidence levels

Facts about agents (formats, locations, behaviour) are gathered by reading local files and sources
read-only; the agents' own binaries are not driven by the repo.

| Level | Meaning |
|---|---|
| VERIFIED | Reproduced programmatically against real data, or pinned by a named test in this repo |
| OBSERVED | Seen in a real file, source or binary string; not reproduced by a program |
| INFERRED | Deduced from data or code; cannot be tested without running the agent |
| `[UNVERIFIED]` | From memory of upstream behaviour or not testable read-only; confirm before relying on it |

A fact moves up a level only with evidence: a test, a staged real-agent check recorded on the
agent page, or a read of the agent's source.

## Rules

- MUST: Every number or claim names its source: a repo path, a test name, or an evidence level plus
  where it was seen.
- MUST: Unverified claims carry the literal tag `[UNVERIFIED]`.
- MUST: A rule comes with the reason or the example that justifies it.
- MUST: Pages describe the current state only; no change logs, no narration of what a thing used
  to be. Update the page and its `Last updated`.
- MUST: No personal data: placeholder paths `/Users/x/...`, projects named `app` or `acme`, no
  contents of real files.
- MUST: Every path in backticks that points into the repo exists.
- SHOULD: A page lets the reader decide something; anecdotes without an action are cut.
- SHOULD: A disproved claim is deleted, not struck through.
- SHOULD: No placeholder or empty pages.

## Source anchors

- `wiki/WIKI-CREATOR.md`
- `wiki/_templates/PAGE.md`
- `crates/core/src/testkit.rs` (fixture builders that make test claims reproducible)

## Verification checklist

- [ ] Every repo path in backticks exists
- [ ] Every number names its source
- [ ] Every agent fact has an evidence level
- [ ] The page is in the `wiki/README.md` index

## Related docs

- `wiki/WIKI-CREATOR.md`
- `wiki/CLAUDE.md`
- `wiki/README.md`

## Last updated

2026-11-10
