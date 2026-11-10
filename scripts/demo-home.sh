#!/bin/bash
# Builds a synthetic agent environment under <dir>: fake homes for Claude Code, Codex, Pi and
# OpenCode plus a state directory, filled with generic memory items. With a nativemem binary
# it runs `sync --apply` against those roots so the other agents hold replicas.
# Usage: scripts/demo-home.sh <dir> [path-to-nativemem-binary]
set -euo pipefail

if [ $# -lt 1 ] || [ $# -gt 2 ]; then
    echo "usage: scripts/demo-home.sh <dir> [path-to-nativemem-binary]" >&2
    exit 2
fi

mkdir -p "$1"
BASE="$(cd "$1" && pwd)"
BIN="${2:-}"
if [ -n "$BIN" ]; then
    case "$BIN" in /*) ;; *) BIN="$PWD/$BIN" ;; esac
    [ -x "$BIN" ] || { echo "error: $BIN is not an executable file" >&2; exit 1; }
fi

CLAUDE="$BASE/claude"
CODEX="$BASE/codex"
PI="$BASE/pi"
OPENCODE="$BASE/opencode"
STATE="$BASE/state"
for d in "$CLAUDE" "$CODEX" "$PI" "$OPENCODE" "$STATE"; do
    case "$d" in "$BASE"/*) ;; *) exit 1 ;; esac
    mkdir -p "$d"
done

# Project folders are fictional; nothing is created at these paths.
WEB="/Users/x/dev/acme/web"
API="/Users/x/dev/acme/api"
BLOG="/Users/x/dev/personal/blog"
ALGO="/Users/x/dev/school/algorithms"

# Claude names a project directory after its path, every non-alphanumeric character becoming "-".
project_dir() {
    printf '%s' "$1" | sed 's/[^A-Za-z0-9]/-/g'
}

for cwd in "$WEB" "$API" "$BLOG" "$ALGO"; do
    dir="$CLAUDE/projects/$(project_dir "$cwd")"
    mkdir -p "$dir/memory"
    printf '{"cwd":"%s"}\n' "$cwd" > "$dir/session.jsonl"
    printf '# Memory Index\n\n' > "$dir/memory/MEMORY.md"
done

# mem <project path> <file slug> <category> <name> <description> <body> <why> <how to apply>
COUNT=0
mem() {
    local cwd="$1" slug="$2" category="$3" name="$4" description="$5" body="$6" why="$7" how="$8"
    local mem_dir="$CLAUDE/projects/$(project_dir "$cwd")/memory"
    local file="${category}_${slug}.md"
    {
        printf -- '---\nname: %s\ndescription: %s\nmetadata:\n  type: %s\n  node_type: memory\n  modified: 2026-01-15\n---\n\n' \
            "$name" "$description" "$category"
        printf '%s\n\n**Why:** %s\n\n**How to apply:** %s\n' "$body" "$why" "$how"
    } > "$mem_dir/$file"
    printf -- '- [%s](%s) — %s\n' "$name" "$file" "$description" >> "$mem_dir/MEMORY.md"
    COUNT=$((COUNT + 1))
}

# ---- acme/web
mem "$WEB" role user "Frontend lead on the web app" \
    "The user owns the acme web frontend and reviews every UI change" \
    "The user is the frontend lead for acme/web and has deep React experience but little backend background." \
    "Explanations about the API layer need more context than explanations about components." \
    "Skip basics for React topics; spell out server-side concepts briefly."
mem "$WEB" tests feedback "Component tests use Testing Library" \
    "Component tests query by role and text, never by class name or test id" \
    "Write component tests with Testing Library queries by accessible role and visible text." \
    "Class-name selectors broke on every styling refactor and hid accessibility problems." \
    "Use getByRole and getByText first; reach for a test id only when no accessible query exists."
mem "$WEB" no-snapshots feedback "No snapshot tests for components" \
    "Snapshot tests were rejected after noisy diffs in review" \
    "Do not add snapshot tests for components; assert on behaviour instead." \
    "Large snapshot diffs were approved without being read, so they caught nothing." \
    "When asked for test coverage, write explicit assertions on rendered output and interactions."
mem "$WEB" design-tokens reference "Design tokens live in packages/ui" \
    "Colors, spacing and type scale come from the shared token package" \
    "All visual constants are defined in packages/ui/tokens and exported as CSS variables." \
    "Hard-coded values drift from the design file and break dark mode." \
    "Use a token for any color or spacing value; add a token instead of a literal."
mem "$WEB" release-freeze project "Release freeze before the spring launch" \
    "Only bug fixes merge to main during the freeze weeks" \
    "A feature freeze applies to acme/web ahead of the spring launch; only fixes are merged." \
    "The launch date is fixed and QA needs a stable build." \
    "Suggest deferring feature work to a branch and keep changes on main minimal and reversible."
mem "$WEB" bundle-size project "Bundle budget of 250 kB per route" \
    "Each route must stay under 250 kB of compressed JavaScript" \
    "The performance budget for acme/web is 250 kB compressed JavaScript per route." \
    "Mobile users on slow connections were the main source of drop-off." \
    "Check the bundle report after adding a dependency and prefer dynamic imports for heavy widgets."
mem "$WEB" analytics reference "Analytics events are documented in the tracking plan" \
    "Event names and properties are defined in docs/tracking-plan.md" \
    "Every analytics event must exist in docs/tracking-plan.md before it is emitted." \
    "Unplanned events made dashboards inconsistent." \
    "Add the event to the tracking plan in the same change that emits it."
mem "$WEB" a11y feedback "Accessibility is part of done" \
    "Keyboard navigation and focus order are checked before a UI task is finished" \
    "A UI change is finished only when it works with the keyboard and keeps a visible focus ring." \
    "Two earlier releases shipped components that could not be reached without a mouse." \
    "Tab through the new UI once and mention the result in the summary."
mem "$WEB" i18n reference "Strings go through the i18n catalog" \
    "User-facing text lives in locales/en.json and is referenced by key" \
    "No user-facing literal strings in components; add a key to locales/en.json." \
    "Translators work from the catalog only." \
    "Add the English string to the catalog and reference it with the t() helper."
mem "$WEB" pr-size feedback "Keep pull requests small" \
    "Pull requests stay under roughly 400 changed lines" \
    "Split work into pull requests of about 400 lines or fewer, each reviewable on its own." \
    "Large pull requests wait days for review and hide regressions." \
    "Propose a split plan before starting a large change."

# ---- acme/api
mem "$API" stack reference "Service stack and layout" \
    "The api is a Go service with PostgreSQL and a thin HTTP layer" \
    "acme/api is written in Go; handlers live in internal/http, queries in internal/store." \
    "New code should follow the existing layering." \
    "Put SQL only in internal/store and keep handlers free of query strings."
mem "$API" migrations feedback "Migrations are additive first" \
    "Schema changes ship in two steps: add, then remove after the deploy" \
    "Never drop or rename a column in the same release that stops using it." \
    "Rolling deploys run old and new code side by side." \
    "Write the additive migration first and note the cleanup migration as a follow-up."
mem "$API" errors feedback "Wrap errors with context" \
    "Errors returned from store calls carry the operation name" \
    "Wrap returned errors with fmt.Errorf and the operation, using %w." \
    "Logs without context made incidents slow to trace." \
    "Add the verb and the entity to every wrapped error, for example 'load invoice: %w'."
mem "$API" rate-limits project "Public endpoints need rate limits" \
    "A rate limiter is required before any endpoint becomes public" \
    "New public endpoints must sit behind the shared rate limiter middleware." \
    "An unauthenticated endpoint was scraped heavily last quarter." \
    "Check for the middleware in the route registration when reviewing endpoint changes."
mem "$API" oncall reference "On-call runbook location" \
    "The runbooks for incidents are in docs/runbooks" \
    "Incident procedures live in docs/runbooks, one file per alert." \
    "On-call engineers read these under pressure, so they must stay accurate." \
    "Update the matching runbook when an alert or its remediation changes."
mem "$API" integration feedback "Integration tests hit a real database" \
    "Store tests run against a disposable PostgreSQL, not mocks" \
    "Test the store package against a real PostgreSQL started by the test helper." \
    "Mocked queries passed while the real SQL was wrong." \
    "Use the testdb helper and keep each test in its own transaction."
mem "$API" deadline project "Billing export due at the end of the quarter" \
    "The invoice export for finance must be live by the quarter end" \
    "Finance needs the invoice export endpoint before the quarter closes." \
    "Manual exports take the finance team two days each month." \
    "Prioritise the export over refactors and flag scope that would delay it."
mem "$API" openapi reference "OpenAPI spec is generated" \
    "api/openapi.yaml is generated from handler annotations" \
    "Do not edit api/openapi.yaml by hand; run make openapi after changing annotations." \
    "Hand edits are overwritten by the next generation." \
    "Change the annotations and regenerate."
mem "$API" logging feedback "Structured logs only" \
    "Log lines are key-value pairs, never formatted sentences" \
    "Use the structured logger with key-value fields; no fmt-style log lines." \
    "The log pipeline indexes fields and drops free text." \
    "Pass ids and counts as fields and keep the message a short constant."
mem "$API" user-role user "Backend engineer, new to the frontend" \
    "The user writes most of the api and is learning the web client" \
    "The user is a backend engineer on acme/api and is still learning frontend tooling." \
    "Frontend explanations should map to backend concepts where possible." \
    "Compare client state and caching to server-side equivalents when explaining."

# ---- personal/blog
mem "$BLOG" stack reference "Blog is a static site" \
    "The blog is built with a static site generator and deployed to a CDN" \
    "personal/blog is a static site; posts are Markdown files under content/posts." \
    "There is no server to configure, so every change is a content or template change." \
    "Edit Markdown and templates only; do not suggest server-side features."
mem "$BLOG" voice feedback "Plain, first-person writing voice" \
    "Posts are written in a plain first-person voice without marketing words" \
    "Write and edit posts in a direct first-person voice; avoid buzzwords and exclamation marks." \
    "The user dislikes text that sounds promotional." \
    "When suggesting edits, shorten sentences and remove filler instead of adding flourish."
mem "$BLOG" images feedback "Images need alt text and a width" \
    "Every image gets descriptive alt text and explicit dimensions" \
    "Add alt text and width and height to every image in a post." \
    "Missing dimensions cause layout shift and missing alt text fails the accessibility check." \
    "Check images when reviewing a draft and ask for a description if none exists."
mem "$BLOG" cadence project "One post every two weeks" \
    "The publishing goal is a post every other Sunday" \
    "The user aims to publish one post every second Sunday." \
    "A steady cadence matters more to the user than post length." \
    "Prefer scoping a draft to finish in one sitting over a long series."
mem "$BLOG" drafts reference "Drafts live in content/drafts" \
    "Unpublished posts stay in content/drafts and are excluded from the build" \
    "Drafts are Markdown files in content/drafts; moving one to content/posts publishes it." \
    "The build only reads content/posts for the public site." \
    "Create new drafts in content/drafts and move them only when the user says to publish."
mem "$BLOG" rss reference "Feed is generated at build time" \
    "The RSS feed is produced by the generator and must not be edited" \
    "feed.xml is generated at build time from post front matter." \
    "A hand-edited feed would be overwritten on the next build." \
    "Fix feed problems through post front matter or the feed template."
mem "$BLOG" user-role user "Writes for a small technical audience" \
    "Readers are developers who like worked examples" \
    "The audience for the blog is working developers who prefer concrete, runnable examples." \
    "Abstract advice without code gets little engagement." \
    "Include a short, complete example in every technical post outline."
mem "$BLOG" links feedback "Link to primary sources" \
    "Claims about tools link to official documentation" \
    "When a post states how a tool behaves, link the official documentation." \
    "Readers check claims and second-hand sources age badly." \
    "Ask for the documentation link when a draft states a behaviour without one."

# ---- school/algorithms
mem "$ALGO" course project "Algorithms course assignments" \
    "Weekly assignments are in Python and graded on correctness and analysis" \
    "school/algorithms holds weekly assignments written in Python; each needs a short complexity analysis." \
    "Grading splits points between passing tests and the written analysis." \
    "Include the time and space complexity of each solution in a comment block."
mem "$ALGO" explain feedback "Explain the idea before the code" \
    "Answers start with the intuition and then show the implementation" \
    "Explain the idea in two or three sentences before giving code." \
    "The user wants to understand the technique, not copy a solution." \
    "Start with the invariant or recurrence, then the code, then the complexity."
mem "$ALGO" no-solutions feedback "Do not hand over full graded solutions" \
    "For graded assignments give hints and checks, not finished answers" \
    "On assignments, guide with hints and review the user's attempt instead of writing the full solution." \
    "Academic integrity rules apply to graded work." \
    "Offer a hint or a test case first and review code the user wrote."
mem "$ALGO" python reference "Python version and test runner" \
    "The course uses Python 3.12 and pytest" \
    "Assignments run on Python 3.12; tests are pytest files named test_*.py." \
    "The autograder uses the same versions." \
    "Write code compatible with 3.12 and run pytest before declaring a task done."
mem "$ALGO" exam project "Midterm covers graphs and dynamic programming" \
    "The midterm topics are shortest paths, spanning trees and dynamic programming" \
    "The upcoming midterm covers graph algorithms and dynamic programming." \
    "The user is planning revision time around those topics." \
    "Favour practice problems on those topics when asked what to study."
mem "$ALGO" notation feedback "Use the course's notation" \
    "Big-O notation and array indexing follow the lecture notes" \
    "Use zero-based indexing and the notation from the lecture notes in explanations." \
    "Mixing conventions confuses the user when comparing with the slides." \
    "State indexing explicitly whenever a recurrence is written down."
mem "$ALGO" user-role user "Second-year computer science student" \
    "The user is a student and prefers step-by-step reasoning" \
    "The user is a second-year computer science student who values step-by-step reasoning." \
    "Skipped steps in derivations lose the user." \
    "Show intermediate steps in proofs and traces of small examples."
mem "$ALGO" visual reference "Visualisations help with graph problems" \
    "Small ASCII diagrams make graph traversals easier to follow" \
    "ASCII diagrams of small graphs help the user follow traversals." \
    "The user learns graph algorithms visually." \
    "Draw a five-node example when explaining a graph algorithm."

# ---- global instructions
cat > "$CLAUDE/CLAUDE.md" <<'EOF'
# Global instructions

## Communication

Be concise and direct. State the result first, then the reasoning if it is not obvious.

## Code style

Prefer small functions and clear names. Match the formatting already used in the file.

## Git

Commit messages use `type(scope): summary`. Do not rewrite shared history.

## Safety

Ask before deleting files or running commands that cannot be undone.
EOF

echo "Created $COUNT memory items in 4 Claude Code projects under $BASE"

if [ -n "$BIN" ]; then
    "$BIN" --claude-home "$CLAUDE" --codex-home "$CODEX" --pi-home "$PI" \
        --opencode-home "$OPENCODE" --state-dir "$STATE" sync --apply
fi

cat <<EOF

Use these roots with the app and the CLI:

export NATIVEMEM_CLAUDE_HOME="$CLAUDE"
export NATIVEMEM_CODEX_HOME="$CODEX"
export NATIVEMEM_PI_HOME="$PI"
export NATIVEMEM_OPENCODE_HOME="$OPENCODE"
export NATIVEMEM_STATE_DIR="$STATE"
EOF
