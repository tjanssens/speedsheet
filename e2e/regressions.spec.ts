// Plain regression tests for bugs the UI fuzz test (ui-fuzz.spec.ts) found, as minimal cases.
import { copyFileSync, mkdirSync } from "node:fs";
import { resolve } from "node:path";
import { expect, test } from "@playwright/test";

test("a slow undo does not pull the cursor away from where the user clicked meanwhile", async ({ page }) => {
  mkdirSync("target/e2e", { recursive: true });
  const file = resolve("target/e2e/regressions.xlsx");
  copyFileSync("testdata/styled-members.xlsx", file);
  page.on("dialog", (d) => void (d.type() === "prompt" ? d.accept(file) : d.dismiss()));
  await page.goto("/");
  await page.waitForFunction(() => "__speedsheet" in window);
  const scroller = page.locator("#scroller");
  const nameBox = page.locator("#name-box");
  await scroller.focus();
  await page.keyboard.press("Control+o");
  await expect(page.locator("#tabs")).toContainText("Leden 2026");

  // An edit in B3 to undo later.
  await nameBox.fill("B3");
  await nameBox.press("Enter");
  await page.keyboard.type("changed");
  await page.keyboard.press("Enter");
  await expect(nameBox).toHaveValue("B4");

  // The undo answers late (a big file); meanwhile the user moves to D7.
  let release = () => {};
  await page.route("**/undo?*", async (route) => {
    await new Promise<void>((done) => (release = done));
    await route.continue();
  });
  await page.keyboard.press("Control+z");
  await nameBox.fill("D7");
  await nameBox.press("Enter");
  await expect(nameBox).toHaveValue("D7");
  const undone = page.waitForResponse(/\/undo\?/);
  release();
  await undone;
  await page.waitForFunction(() => (window as unknown as { __speedsheet: { state(): { callsInFlight: number } } }).__speedsheet.state().callsInFlight === 0);
  await expect(nameBox).toHaveValue("D7");
});
