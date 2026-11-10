# <Agent name>

## Purpose

<What nativemem needs to know about this agent to read it, write it, or decide not to.>

## Scope

<Which agent versions the facts were gathered on, and which resource kinds the page covers.>

## Status in nativemem

<One line per kind (memory, instruction sections, sessions): read, write, or not implemented.
State the current fact only. Name the adapter file, or say plainly that no adapter exists.>

## Native locations

| What | Global | Project |
|---|---|---|
| Instructions | | |
| Memory | | |
| Sessions | | |
| MCP servers | | |
| Skills | | |

## Formats

<Memory, instructions, sessions, MCP, skills: the shape of each file or table, as far as it is known.>

## What nativemem reads and writes today

<Paths and markers. How replicas are recognised. What is never touched.>

## Sessions: listing and resume

<How the agent builds its resume list and what a written session needs to appear in it.>

## Quirks

<Behaviour that bit us or will: lossy names, rewritten config files, caches, pinned hashes.>

## Version detection

<How the agent version is found, and what nativemem pins in code.>

## Facts and evidence

| Fact | Evidence level | Source |
|---|---|---|
| | VERIFIED / OBSERVED / INFERRED / `[UNVERIFIED]` | |

## Source anchors

- `<repo path or test name>`

## Rules

- MUST: <rule>
- SHOULD: <rule>

## Verification checklist

- [ ] Every repo path in backticks exists
- [ ] Every row of the facts table names its evidence level
- [ ] The README support table and `wiki/README.md` agree with this page

## Related docs

- `wiki/guides/add-an-agent.md`

## Last updated

YYYY-MM-DD
