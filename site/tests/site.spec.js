import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

test("assets load locally, internal links resolve and the product claims are explicit", async ({
  page,
}) => {
  const failed = [];
  page.on("pageerror", (error) => failed.push(error.message));
  page.on("response", (response) => {
    if (response.status() >= 400) failed.push(response.url());
  });
  await page.goto("/");
  await expect(page).toHaveTitle(
    "nativemem — Change the agent. Keep the context.",
  );
  await expect(page.getByRole("heading", { level: 1 })).toHaveText(
    /Change the agent\.\s*Keep the\s+context\s*\./,
  );
  const localErrors = await page.evaluate(async () => {
    const errors = [];
    for (const link of document.querySelectorAll('a[href^="#"]')) {
      const id = link.getAttribute("href").slice(1);
      if (id && !document.getElementById(id)) errors.push(`missing #${id}`);
    }
    for (const image of document.querySelectorAll("img")) {
      const response = await fetch(image.src);
      if (!response.ok) errors.push(`image ${image.src}`);
      if (!image.hasAttribute("alt")) errors.push(`missing alt ${image.src}`);
    }
    return errors;
  });
  expect(localErrors).toEqual([]);
  await expect(page.getByText("Windows isn't supported yet.")).toBeVisible();
  expect(failed).toEqual([]);
});

test("supported agents use local logo assets with stable sizing", async ({
  page,
}) => {
  await page.goto("/");
  const strip = page.getByRole("list", { name: "Supported coding agents" });
  await expect(strip.locator("li")).toHaveText([
    "Claude Code", "Codex", "OpenCode", "Pi",
  ]);
  const logos = strip.locator("img.agent-logo");
  await expect(logos).toHaveCount(4);
  const files = ["claude.svg", "codex.png", "opencode.svg", "pi.png"];
  for (const [index, file] of files.entries()) {
    const logo = logos.nth(index);
    await expect(logo).toHaveAttribute("src", `assets/agents/${file}`);
    await expect(logo).toHaveAttribute("alt", "");
    await expect(logo).toHaveAttribute("aria-hidden", "true");
    await expect(logo).toHaveAttribute("width", "28");
    await expect(logo).toHaveAttribute("height", "28");
    await expect
      .poll(() => logo.evaluate((image) => image.complete && image.naturalWidth > 0), {
        message: `${file} must decode as an image`,
      })
      .toBe(true);
  }
  await page.emulateMedia({ colorScheme: "dark" });
  for (const logo of await logos.all()) {
    await expect(logo).toHaveCSS("color-scheme", "light");
  }
});

test("Pi artwork has transparent cutouts and preserves the reference colors", async ({ page }) => {
  await page.goto("/");
  const logo = page.locator('.agent-logo[src="assets/agents/pi.png"]');
  const pixels = await logo.evaluate(async (image) => {
    await image.decode();
    const canvas = document.createElement("canvas");
    canvas.width = image.naturalWidth;
    canvas.height = image.naturalHeight;
    const context = canvas.getContext("2d");
    context.drawImage(image, 0, 0);
    return [[100, 100], [250, 20], [26, 26], [26, 236], [236, 236]].map(
      ([x, y]) => Array.from(context.getImageData(x, y, 1, 1).data),
    );
  });
  expect(pixels[0][3]).toBe(0);
  expect(pixels[1][3]).toBe(0);
  expect(pixels.slice(2)).toEqual([
    [240, 144, 130, 255],
    [77, 154, 191, 255],
    [241, 190, 88, 255],
  ]);
});

test("Mac downloads target the universal DMG with a ZIP alternative", async ({
  page,
}) => {
  await page.goto("/");
  const downloads = page.getByRole("link", {
    name: "Download for Mac",
    exact: true,
  });
  await expect(downloads).toHaveCount(2);
  for (const link of await downloads.all()) {
    await expect(link).toHaveAttribute(
      "href",
      "https://github.com/efekurucay/nativemem/releases/latest/download/nativemem-macos-universal.dmg",
    );
  }
  await expect(
    page.getByRole("link", { name: "ZIP alternative", exact: true }),
  ).toHaveAttribute(
    "href",
    "https://github.com/efekurucay/nativemem/releases/latest/download/nativemem-macos-universal.zip",
  );
  await expect(page.getByRole("paragraph").filter({ hasText: "To install:" })).toContainText(
    "not notarized",
  );
});

test("screenshot tabs support clicks and keyboard navigation", async ({
  page,
}) => {
  await page.goto("/");
  await page.evaluate(() => document.fonts.ready);
  const skills = page.getByRole("tab", { name: "Skill locations" });
  await skills.click();
  await expect(skills).toHaveAttribute("aria-selected", "true");
  await expect(page.getByRole("tabpanel")).toHaveCount(1);
  await expect(page.getByRole("tabpanel")).toContainText("exact paths");
  await skills.press("ArrowLeft");
  await expect(page.getByRole("tab", { name: "Project memory" })).toBeFocused();
  await expect(page.getByRole("tabpanel")).toContainText(
    "Memory stays with its project",
  );
  await page.getByRole("tab", { name: "Project memory" }).press("Home");
  await expect(page.getByRole("tab", { name: "Overview" })).toBeFocused();
});

test("installation commands copy to the clipboard", async ({
  page,
  context,
}) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await page.goto("/");
  await page
    .getByRole("button", { name: "Copy source installation commands" })
    .click();
  await expect(page.getByRole("status")).toHaveText(
    "Installation commands copied.",
  );
  const text = await page.evaluate(() => navigator.clipboard.readText());
  expect(text).toContain("cargo install --path crates/cli");
  expect(text).toContain("nativemem sync --apply");
});

test("keyboard users can skip navigation and read the FAQ", async ({
  page,
}) => {
  await page.goto("/");
  await page.keyboard.press("Tab");
  await expect(
    page.getByRole("link", { name: "Skip to content" }),
  ).toBeFocused();
  await page.getByText("What actually syncs?", { exact: true }).click();
  await expect(
    page.getByText("Project skills are implemented", { exact: false }),
  ).toBeVisible();
});

for (const width of [360, 390, 768, 1024, 1440]) {
  test(`layout and accessibility at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 1000 });
    await page.goto("/");
    await page.evaluate(() => document.fonts.ready);
    await expect
      .poll(() =>
        page.evaluate(() => {
          if (document.documentElement.scrollWidth <= window.innerWidth)
            return [];
          return [
            `viewport ${innerWidth}, document ${document.documentElement.scrollWidth}`,
            ...[...document.querySelectorAll("body *")]
              .filter(
                (element) =>
                  element.getBoundingClientRect().right > innerWidth + 0.5,
              )
              .map(
                (element) =>
                  `${element.tagName}.${element.className}: ${element.getBoundingClientRect().right}`,
              ),
          ];
        }),
      )
      .toEqual([]);
    const results = await new AxeBuilder({ page })
      .withTags(["wcag2a", "wcag2aa", "wcag21aa"])
      .analyze();
    expect(results.violations).toEqual([]);
  });
}

test("core information is readable with JavaScript disabled", async ({
  browser,
}) => {
  const context = await browser.newContext({ javaScriptEnabled: false });
  const page = await context.newPage();
  await page.goto("http://127.0.0.1:4173");
  await expect(page.getByRole("heading", { level: 1 })).toBeVisible();
  await page.getByText("How finished is it?", { exact: true }).click();
  await expect(
    page.getByText("The Mac app and Rust CLI are under active development.", {
      exact: false,
    }),
  ).toBeVisible();
  await context.close();
});
