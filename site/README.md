# nativemem landing page

Static HTML, CSS and JavaScript. No build step, analytics, cookies or runtime packages. Fonts are
self-hosted; their OFL licenses are in `assets/fonts/`. All desktop screenshots use synthetic data,
not a user's agent homes. `assets/social.png` is the social sharing card.

## Agent logos

`assets/agents/` holds publisher marks and the user-supplied Pi artwork, served locally without
third-party requests. Pi's outer white margin is cropped; its colors and artwork are preserved. Visible agent names label the decorative images; fixed dimensions reserve space.
The strip stays light, including embedded SVGs when the browser prefers dark colors.

Sources:

- `claude.svg`: [Claude's official site icon](https://claude.com/icon.svg).
- `codex.png`: OpenAI's [official Codex extension](https://marketplace.visualstudio.com/items?itemName=openai.chatgpt),
  [published icon](https://openai.gallerycdn.vsassets.io/extensions/openai/chatgpt/26.51106.51428/1794372152150/Microsoft.VisualStudio.Services.Icons.Default).
- `opencode.svg`: [OpenCode's official repository](https://github.com/anomalyco/opencode/blob/dev/packages/ui/src/assets/favicon/favicon-v3.svg).
- `pi.png`: [user-supplied color Pi reference](https://encrypted-tbn0.gstatic.com/images?q=tbn:ANd9GcRF2bgJAlOiyYcfdqBG8VSAhIbrFlg_GGPXwNR04jfXS4Z_cDvDllNBQlE&s=10).

These marks identify supported integrations and remain their respective owners' trademarks.

## Preview

```sh
cd site
python3 -m http.server 4173 --bind 127.0.0.1
```

Open <http://127.0.0.1:4173>. Relative URLs let the site run at a domain root or a project subpath.
Do not serve the repository root publicly; only this directory is a website.

## Browser tests

Node 22+, Python 3 and Chromium:

```sh
npm ci
npx playwright install chromium
npm test
```

The Playwright suite checks screenshot tabs (including keyboard controls), clipboard copying,
internal anchors, images, agent logos and their sizing, a no-JavaScript view, horizontal
overflow at five viewport widths, and WCAG A/AA rules with axe. `tests/server.py` serves the fixture
with the OS connection-backlog limit so parallel browser connection pools can load assets together.
No external service or real agent is contacted.

## Desktop icon

`npm run icon` renders `assets/mark.svg` into `../apps/macos/Resources/AppIcon.png`, with a transparent
margin for the macOS icon grid. It uses the same Chromium installation as the browser tests.
The Mac bundle script creates its ICNS sizes from this PNG; no browser is bundled with the app.

## Publish

`.github/workflows/site.yml` tests and publishes this directory to GitHub Pages on pushes to `main`.
Pull requests run checks only. The workflow can also be dispatched manually.

The repository's Pages source must be **GitHub Actions** (Settings → Pages). The deployment job
returns the live URL. Only `index.html`, `style.css`, `app.js` and `assets/` are uploaded: no tests,
`node_modules`, lockfiles, source-tree files or agent data. A custom domain is not configured here.

## Keep the claims accurate

The support matrix lives in the root README and the wiki's per-agent pages. Keep platform limits,
project-skill support, conversion limits and source-install instructions consistent with them.
The primary Mac download is `releases/latest/download/nativemem-macos-universal.dmg`; the ZIP is an
alternative. These names come from `scripts/release.sh`. Publish and verify the release assets before
shipping a new download link. Keep the macOS minimum, universal architecture and notarization warning
next to installation instructions. Do not publish test counts as benchmarks
or promise that schema tests verify native session resume.
