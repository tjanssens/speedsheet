// Smoke test of the whole stack: UI -> API -> core -> UI. It opens a real xlsx, edits through
// the keyboard, undoes, enters a formula, and fails on any uncaught error in the page.
import { copyFileSync, mkdirSync } from "node:fs";
import { resolve } from "node:path";
import { expect, test } from "@playwright/test";

test("open, edit, undo and formula round trip", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  // Open its own copy through the dev bridge's "open" prompt (other tests may have run first).
  mkdirSync("target/e2e", { recursive: true });
  const file = resolve("target/e2e/smoke.xlsx");
  copyFileSync("testdata/styled-members.xlsx", file);
  page.on("dialog", (d) => {
    if (d.type() === "prompt") return void d.accept(file);
    errors.push(`dialog: ${d.message()}`);
    void d.dismiss();
  });

  // Edits can be held back to finish later, as on a big file where recalculation takes time.
  let holdNextEdit = false;
  let releaseEdit = () => {};
  await page.route(/\/set_inputs\?/, async (route) => {
    if (holdNextEdit) {
      holdNextEdit = false;
      await new Promise<void>((done) => (releaseEdit = done));
    }
    await route.continue();
  });

  await page.goto("/");
  await page.waitForFunction(() => "__speedsheet" in window);
  await page.locator("#scroller").focus();
  await page.keyboard.press("Control+o");
  await expect(page.locator("#tabs")).toContainText("Leden 2026");
  const nameBox = page.locator("#name-box");
  const contents = page.locator("#formula-input");
  const goTo = async (addr: string) => {
    await nameBox.fill(addr);
    await nameBox.press("Enter");
    await expect(nameBox).toHaveValue(addr);
  };

  // Read a cell from the file (testdata/styled-members.xlsx, made by examples/gen_styled.rs).
  await goTo("B3");
  const original = " Tawfik";
  await expect(contents).toHaveValue(original);

  // Type over it, Enter moves down, come back: the new value is stored.
  await page.locator("#scroller").focus();
  await page.keyboard.type("Edited from e2e");
  await page.keyboard.press("Enter");
  await expect(nameBox).toHaveValue("B4");
  await goTo("B3");
  await expect(contents).toHaveValue("Edited from e2e");

  // Undo brings the original back.
  await page.locator("#scroller").focus();
  await page.keyboard.press("Control+z");
  await expect(contents).toHaveValue(original);

  // A formula in an empty cell keeps its text. The edit finishes while the next address is
  // being typed in the name box: it must not overwrite that address.
  await goTo("K20");
  await page.locator("#scroller").focus();
  holdNextEdit = true;
  await page.keyboard.type("=1+2");
  await page.keyboard.press("Enter");
  await expect(nameBox).toHaveValue("K21");
  await nameBox.fill("K20");
  const editDone = page.waitForResponse(/set_inputs/);
  releaseEdit();
  await editDone;
  await nameBox.press("Enter");
  await expect(nameBox).toHaveValue("K20");
  await expect(contents).toHaveValue("=1+2");

  await page.screenshot({ path: "test-results/smoke.png" });
  expect(errors, "uncaught errors in the page").toEqual([]);
});
