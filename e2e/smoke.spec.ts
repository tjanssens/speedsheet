// Smoke test of the whole stack: UI -> API -> core -> UI. It opens a real xlsx, edits through
// the keyboard, undoes, enters a formula, and fails on any uncaught error in the page.
import { expect, test } from "@playwright/test";

test("open, edit, undo and formula round trip", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  page.on("dialog", (d) => {
    errors.push(`dialog: ${d.message()}`);
    void d.dismiss();
  });

  await page.goto("/");
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

  // A formula in an empty cell keeps its text.
  await goTo("K20");
  await page.locator("#scroller").focus();
  await page.keyboard.type("=1+2");
  await page.keyboard.press("Enter");
  await expect(nameBox).toHaveValue("K21");
  await goTo("K20");
  await expect(contents).toHaveValue("=1+2");

  await page.screenshot({ path: "test-results/smoke.png" });
  expect(errors, "uncaught errors in the page").toEqual([]);
});
