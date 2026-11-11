# nativemem landing page

Static HTML, CSS and JavaScript. No build step, analytics, cookies or runtime packages. Fonts are
self-hosted; their OFL licenses are in `assets/fonts/`. All desktop screenshots use synthetic data,
not a user's agent homes. `assets/social.png` is the social sharing card.

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
internal anchors, images, a no-JavaScript view, horizontal overflow at five viewport widths, and
WCAG A/AA rules with axe. No external service or real agent is contacted.

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
Download links go to Releases, not a guessed binary URL. Do not publish test counts as benchmarks
or promise that schema tests verify native session resume.
