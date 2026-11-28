// Copy, cut, paste and clear. A copy inside SpeedSheet keeps the formulas (shifted like Excel);
// a cut moves the original inputs.
import { call, isTauri, type Copied, type EditResult } from "./api";
import { app, grid, selectionClipped } from "./app";
import { afterEdit } from "./book";
import { fmtNum, toast } from "./ui";

async function clipboardWrite(c: Copied) {
  if (isTauri) {
    const cb = await import("@tauri-apps/plugin-clipboard-manager");
    try {
      await cb.writeHtml(c.html, c.tsv);
    } catch {
      await cb.writeText(c.tsv);
    }
  } else {
    try {
      await navigator.clipboard.write([
        new ClipboardItem({
          "text/plain": new Blob([c.tsv], { type: "text/plain" }),
          "text/html": new Blob([c.html], { type: "text/html" }),
        }),
      ]);
    } catch {
      await navigator.clipboard.writeText(c.tsv);
    }
  }
}

async function clipboardRead(): Promise<string> {
  if (isTauri) {
    const cb = await import("@tauri-apps/plugin-clipboard-manager");
    return (await cb.readText()) ?? "";
  }
  return navigator.clipboard.readText();
}

export async function copy(cut = false) {
  const r = selectionClipped();
  const c = await call<Copied>("copy_range", { sheet: app.sheet, ...r });
  await clipboardWrite(c);
  app.lastCopy = { text: c.tsv, row: r.r0, col: r.c0, inputs: c.inputs, cut, sheet: app.sheet, rect: r };
  grid.copyRect = r;
  grid.render();
  const cells = (r.r1 - r.r0 + 1) * (r.c1 - r.c0 + 1);
  toast(`${cut ? "Cut" : "Copied"} ${fmtNum(cells)} cell${cells === 1 ? "" : "s"}`, 1500);
}

const norm = (s: string) => s.replace(/\r\n/g, "\n").replace(/\n$/, "");

export async function paste() {
  const text = await clipboardRead();
  if (!text) return;
  const { row, col } = grid.active;
  const sheet = app.sheet;
  const internal = app.lastCopy && norm(app.lastCopy.text) === norm(text) ? app.lastCopy : null;
  let res: EditResult;
  if (internal?.cut) {
    // Move: clear the source first, then paste the original inputs unchanged.
    await call<EditResult>("clear_range", { sheet: internal.sheet, ...internal.rect });
    res = await call<EditResult>("paste", { sheet, row, col, text, internal: { row, col, inputs: internal.inputs } });
    app.lastCopy = null;
  } else {
    res = await call<EditResult>("paste", {
      sheet,
      row,
      col,
      text,
      internal: internal ? { row: internal.row, col: internal.col, inputs: internal.inputs } : null,
    });
  }
  grid.copyRect = null;
  afterEdit(res, false);
  const rows = internal ? internal.inputs.length : text.replace(/\r?\n$/, "").split(/\r?\n/).length;
  const cols = internal ? (internal.inputs[0]?.length ?? 1) : text.split(/\r?\n/)[0].split("\t").length;
  grid.select({ row, col }, false, { row: row + rows - 1, col: col + cols - 1 });
}

export async function clearSelection() {
  afterEdit(await call<EditResult>("clear_range", { sheet: app.sheet, ...selectionClipped() }), false);
}

/** Escape: forget the copied range (the marching ants go away). */
export function dropCopy() {
  if (!grid.copyRect) return;
  grid.copyRect = null;
  app.lastCopy = null;
  grid.render();
}
