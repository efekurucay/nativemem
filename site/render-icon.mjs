import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const mark = await readFile(new URL("./assets/mark.svg", import.meta.url), "utf8");
const output = fileURLToPath(
  new URL("../apps/macos/Resources/AppIcon.png", import.meta.url),
);
const browser = await chromium.launch();
try {
  const page = await browser.newPage({
    viewport: { width: 1024, height: 1024 },
    deviceScaleFactor: 1,
  });
  // Keep the mark inside the macOS icon grid, with transparent space around its rounded square.
  await page.setContent(`<!doctype html><html><head><style>
    html, body { margin: 0; width: 1024px; height: 1024px; background: transparent; }
    svg { position: absolute; left: 100px; top: 100px; width: 824px; height: 824px; }
  </style></head><body>${mark}</body></html>`);
  await page.screenshot({ path: output, omitBackground: true });
  console.log(`Rendered ${output}`);
} finally {
  await browser.close();
}
