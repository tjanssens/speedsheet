// Property test for the UI: random clicks, drags, keys and edits on a real workbook, with rules
// that must hold after every step. It finds the kind of bug nobody writes a test for until a
// user reports it (a selection that keeps following the mouse after a missed release, an editor
// left open, a grid that shows something else than the engine has).
//
//   npx playwright test ui-fuzz                       300 steps, a new random seed each run
//   UI_FUZZ_SEED=1234 npx playwright test ui-fuzz     replay a failure (the seed is printed)
//   UI_FUZZ_STEPS=3000 ...                            longer (nightly)
import { copyFileSync, mkdirSync } from "node:fs";
import { resolve } from "node:path";
import { expect, test, type Page } from "@playwright/test";

interface UiState {
  sheet: number;
  active: { row: number; col: number };
  anchor: { row: number; col: number };
  selection: { r0: number; c0: number; r1: number; c1: number };
  rowCount: number;
  colCount: number;
  dragging: boolean;
  editing: null | { mode: string; where: string };
  editorVisible: boolean;
  shownText: string | null;
  pendingDraw: boolean;
  callsInFlight: number;
  calls: { pending: Record<string, number>; lastError: string };
}

const SEED = Number(process.env.UI_FUZZ_SEED ?? Math.floor(Math.random() * 2 ** 31));
const STEPS = Number(process.env.UI_FUZZ_STEPS ?? 300);

/** mulberry32: small, fast and the same in every run for the same seed. */
function rng(seed: number) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const state = (page: Page) => page.evaluate(() => (window as unknown as { __speedsheet: { state(): UiState } }).__speedsheet.state());

const colName = (c: number) => {
  let s = "";
  for (c += 1; c > 0; c = Math.floor((c - 1) / 26)) s = String.fromCharCode(65 + ((c - 1) % 26)) + s;
  return s;
};

test("random use of the grid keeps the UI consistent", async ({ page }) => {
  test.setTimeout(30_000 + STEPS * 400);
  console.log(`UI fuzz seed ${SEED} (replay: UI_FUZZ_SEED=${SEED}), ${STEPS} steps`);
  const rand = rng(SEED);
  const pick = <T>(xs: readonly T[]) => xs[Math.floor(rand() * xs.length)];
  const int = (lo: number, hi: number) => lo + Math.floor(rand() * (hi - lo + 1));

  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  // Its own copy of the styled test file, opened through the dev bridge's "open" prompt.
  mkdirSync("target/e2e", { recursive: true });
  const file = resolve("target/e2e/fuzz.xlsx");
  copyFileSync("testdata/styled-members.xlsx", file);
  page.on("dialog", (d) => {
    if (d.type() === "prompt") void d.accept(file);
    else if (d.type() === "confirm") void d.accept();
    else {
      errors.push(`dialog: ${d.message()}`);
      void d.dismiss();
    }
  });

  await page.goto("/");
  await page.waitForFunction(() => "__speedsheet" in window);
  await page.locator("#scroller").focus();
  await page.keyboard.press("Control+o");
  await expect(page.locator("#tabs")).toContainText("Leden 2026");

  const scroller = page.locator("#scroller");
  const box = (await scroller.boundingBox())!;
  const cellPoint = () => ({ x: box.x + int(60, box.width - 30), y: box.y + int(30, box.height - 30) });

  const actions: { name: string; weight: number; run: () => Promise<void> }[] = [
    { name: "click", weight: 6, run: async () => void (await page.mouse.click(cellPoint().x, cellPoint().y)) },
    {
      name: "shift-click",
      weight: 2,
      run: async () => {
        const p = cellPoint();
        await page.keyboard.down("Shift");
        await page.mouse.click(p.x, p.y);
        await page.keyboard.up("Shift");
      },
    },
    {
      name: "drag",
      weight: 3,
      run: async () => {
        const a = cellPoint();
        const b = cellPoint();
        await page.mouse.move(a.x, a.y);
        await page.mouse.down();
        await page.mouse.move(b.x, b.y, { steps: int(2, 6) });
        await page.mouse.up();
      },
    },
    {
      // The 0.3.4 bug: the release happens outside the window and never arrives.
      name: "drag, release missed",
      weight: 2,
      run: async () => {
        const a = cellPoint();
        const b = cellPoint();
        await page.mouse.move(a.x, a.y);
        await page.mouse.down();
        await page.mouse.move(b.x, b.y, { steps: 3 });
        await page.evaluate(() => window.dispatchEvent(new Event("blur")));
        await page.mouse.up();
        await page.mouse.move(cellPoint().x, cellPoint().y, { steps: 2 });
      },
    },
    { name: "column header", weight: 1, run: async () => void (await page.mouse.click(box.x + int(60, box.width - 30), box.y + 12)) },
    { name: "row header", weight: 1, run: async () => void (await page.mouse.click(box.x + 15, box.y + int(30, box.height - 30))) },
    {
      name: "double-click, then Escape or Enter",
      weight: 2,
      run: async () => {
        const p = cellPoint();
        await page.mouse.dblclick(p.x, p.y);
        await page.keyboard.press(pick(["Escape", "Enter"]));
      },
    },
    {
      name: "type a value",
      weight: 3,
      run: async () => {
        await page.keyboard.type(pick(["7", "x", "=1+2", "=A1", "Zoë", "-3.5"]));
        await page.keyboard.press(pick(["Enter", "Tab", "ArrowDown", "Escape"]));
      },
    },
    {
      name: "key",
      weight: 8,
      run: async () => {
        const mod = pick(["", "", "", "Shift+", "Control+", "Control+Shift+"]);
        const key = pick(["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Enter", "Tab", "PageDown", "PageUp", "Home", "End"]);
        await page.keyboard.press(mod + key);
      },
    },
    { name: "F2 + Escape", weight: 1, run: async () => void (await page.keyboard.press("F2").then(() => page.keyboard.press("Escape"))) },
    { name: "delete", weight: 1, run: async () => void (await page.keyboard.press("Delete")) },
    { name: "undo/redo", weight: 2, run: async () => void (await page.keyboard.press(pick(["Control+z", "Control+y"]))) },
    { name: "select all", weight: 1, run: async () => void (await page.keyboard.press("Control+a")) },
  ];
  const total = actions.reduce((n, a) => n + a.weight, 0);
  const choose = () => {
    let r = rand() * total;
    for (const a of actions) if ((r -= a.weight) < 0) return a;
    return actions[0];
  };

  const log: string[] = [];
  const fail = (msg: string): never => {
    throw new Error(`${msg}\nseed ${SEED}, last steps:\n  ${log.slice(-12).join("\n  ")}`);
  };
  const check = async (what: string, poll: () => Promise<unknown>) => {
    try {
      await poll();
    } catch (e) {
      fail(`${what}: ${e instanceof Error ? e.message : String(e)}\nstate: ${JSON.stringify(await state(page))}`);
    }
  };

  for (let step = 0; step < STEPS; step++) {
    const a = choose();
    log.push(`${step}: ${a.name}`);
    await a.run();
    // Let the UI settle: no engine call outstanding, nothing left to draw.
    await check("the UI settles (no engine call open, no draw pending)", () =>
      page.waitForFunction(
        () => {
          const st = (window as unknown as { __speedsheet: { state(): UiState } }).__speedsheet.state();
          return st.callsInFlight === 0 && !st.pendingDraw;
        },
        undefined,
        { timeout: 20_000 },
      ),
    );
    const s = await state(page);
    if (process.env.UI_FUZZ_TRACE) console.log(log[log.length - 1], JSON.stringify(s.active), JSON.stringify(s.selection), await page.locator("#name-box").inputValue(), JSON.stringify(s.editing));

    // The selection is the rectangle between anchor and active cell, inside the grid.
    const sel = s.selection;
    const expectSel = {
      r0: Math.min(s.anchor.row, s.active.row),
      r1: Math.max(s.anchor.row, s.active.row),
      c0: Math.min(s.anchor.col, s.active.col),
      c1: Math.max(s.anchor.col, s.active.col),
    };
    if (JSON.stringify(sel) !== JSON.stringify(expectSel)) fail(`selection ${JSON.stringify(sel)} is not anchor..active`);
    if (sel.r0 < 0 || sel.c0 < 0 || sel.r1 >= s.rowCount || sel.c1 >= s.colCount) fail(`selection ${JSON.stringify(sel)} outside the grid`);
    // No mouse button is down between steps, so no drag may still be going on.
    if (s.dragging) fail("a drag is still active after the mouse was released");
    // The in-cell editor is shown exactly while a cell is being edited.
    if (s.editorVisible !== (s.editing?.where === "cell")) fail(`editor visible=${s.editorVisible} but editing=${JSON.stringify(s.editing)}`);
    // The name box names the active cell.
    if (!s.editing && sel.r0 === sel.r1 && sel.c0 === sel.c1) {
      const want = `${colName(s.active.col)}${s.active.row + 1}`;
      await check("the name box names the active cell", () => expect.poll(() => page.locator("#name-box").inputValue(), { timeout: 2000 }).toBe(want));
    }
    if (errors.length) fail(`errors in the page: ${errors.join("; ")}`);

    // Every 20 steps: the grid shows what the engine has in the active cell.
    if (step % 20 === 19 && !s.editing) {
      await check("the grid shows what the engine has", () =>
        expect.poll(
          () =>
            page.evaluate(async () => {
              const h = (window as unknown as { __speedsheet: { state(): UiState; call(c: string, a: object): Promise<{ text: string[] }> } }).__speedsheet;
              const st = h.state();
              const b = await h.call("get_cells", { sheet: st.sheet, r0: st.active.row, r1: st.active.row, c0: st.active.col, c1: st.active.col });
              return st.shownText === (b.text[0] ?? "") ? "same" : `grid "${st.shownText}" vs engine "${b.text[0] ?? ""}"`;
            }),
          { timeout: 3000 },
        ).toBe("same"),
      );
    }
  }
  expect(errors, "uncaught errors in the page").toEqual([]);
});
