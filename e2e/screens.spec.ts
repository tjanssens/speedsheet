// Screenshots of the main screens, light and dark, for every PR (CI artifact "ui-screens"):
// review a UI change by looking at it, not by reading canvas code. Compare with the LibreOffice
// rendering of testdata/styled-members.xlsx when formatting changes (CLAUDE.md).
import { copyFileSync, mkdirSync } from "node:fs";
import { resolve } from "node:path";
import { expect, test, type Page } from "@playwright/test";

const OUT = "test-results/screens";

async function openStyled(page: Page, name: string) {
  mkdirSync("target/e2e", { recursive: true });
  const file = resolve(`target/e2e/${name}.xlsx`);
  copyFileSync("testdata/styled-members.xlsx", file);
  page.on("dialog", (d) => void (d.type() === "prompt" ? d.accept(file) : d.dismiss()));
  await page.goto("/");
  await page.waitForFunction(() => "__speedsheet" in window);
  await page.locator("#scroller").focus();
  await page.keyboard.press("Control+o");
  await expect(page.locator("#tabs")).toContainText("Leden 2026");
}

const settle = async (page: Page) => {
  try {
    await page.waitForFunction(
      () => {
        const st = (window as unknown as { __speedsheet: { state(): { callsInFlight: number; pendingDraw: boolean } } }).__speedsheet.state();
        return st.callsInFlight === 0 && !st.pendingDraw;
      },
      undefined,
      { timeout: 10_000 },
    );
  } catch (e) {
    const st = await page.evaluate(() => (window as unknown as { __speedsheet: { state(): unknown } }).__speedsheet.state());
    throw new Error(`the UI did not settle: ${JSON.stringify(st)}`, { cause: e });
  }
};

for (const scheme of ["light", "dark"] as const) {
  test(`screens in ${scheme} mode`, async ({ page }, info) => {
    await page.emulateMedia({ colorScheme: scheme });
    await page.setViewportSize({ width: 1280, height: 720 });
    await openStyled(page, `screens-${scheme}-${info.repeatEachIndex}`);
    mkdirSync(OUT, { recursive: true });
    const shot = async (what: string) => {
      await settle(page);
      await page.screenshot({ path: `${OUT}/${scheme}-${what}.png` });
    };
    await shot("1-sheet");

    // A selection with statistics in the status bar.
    const nameBox = page.locator("#name-box");
    await nameBox.fill("B3");
    await nameBox.press("Enter");
    await page.keyboard.press("Shift+ArrowDown");
    await page.keyboard.press("Shift+ArrowDown");
    await page.keyboard.press("Shift+ArrowRight");
    await shot("2-selection");

    // Typing a formula: colored references, autocomplete.
    await nameBox.fill("K5");
    await nameBox.press("Enter");
    await page.keyboard.type("=SUM(B3:C6)+AV");
    await shot("3-formula-help");
    await page.keyboard.press("Escape");

    // The filter popover.
    await nameBox.fill("B4");
    await nameBox.press("Enter");
    await page.locator('[data-cmd="filter"]').click();
    await settle(page);
    await page.keyboard.press("Escape");
    await page.mouse.click(10, 10);
    await shot("4-filter-buttons");
  });
}
