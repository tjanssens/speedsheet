// New, open, save and undo: the commands that replace or write the whole workbook.
import { call, isTauri, native, type BookInfo, type EditResult } from "./api";
import { app, grid } from "./app";
import { afterEdit, applyInfo } from "./book";
import { commitEdit } from "./editing";
import { askAlreadyOpen, isBlankWindow, refreshPeers, type OpenElsewhere } from "./peers";
import { busy, confirmDialog, escapeHtml, fmtNum, showError, spawn, toast } from "./ui";

const OPEN_EXT = ["xlsx", "xlsm", "xls", "xlsb", "ods", "csv", "tsv", "txt"];

export async function confirmDiscard(): Promise<boolean> {
  // With other windows on the file, the changes stay there.
  if (!app.book?.modified || app.peers.count > 1) return true;
  return confirmDialog(`"${app.book.name}" has unsaved changes. Discard them?`, "Unsaved changes", "Discard");
}

/** Show a different workbook in this window. */
function showBook(info: BookInfo) {
  grid.dropCache();
  grid.showFilterButtons = false;
  app.sheet = -1;
  app.bookGen++;
  applyInfo(info, 0);
  grid.select({ row: 0, col: 0 });
  spawn(refreshPeers());
}

export async function cmdNew() {
  if (!(await confirmDiscard())) return;
  showBook(await call<BookInfo>("new_book"));
}

export async function openPath(path: string) {
  let shared = false;
  const other = await call<OpenElsewhere | null>("find_open", { path }).catch(() => null);
  if (other) {
    const choice = await askAlreadyOpen(other);
    if (choice === "cancel") return;
    if (choice === "switch") {
      const blank = isBlankWindow();
      await native("focus_window", { label: other.view }).catch((e: unknown) => showError(e, "Can't switch windows"));
      // A window that was opened just for this file has no reason to stay.
      if (blank && isTauri) {
        const { getCurrentWindow } = await import("@tauri-apps/api/window");
        await getCurrentWindow().destroy();
      }
      return;
    }
    shared = true;
  }
  if (!(await confirmDiscard())) return;
  const t0 = performance.now();
  try {
    const info = await busy(`Opening ${path.split(/[\\/]/).pop()}…`, () => call<BookInfo>("open_file", { path, shared }));
    showBook(info);
    const rows = info.sheets.reduce((n, s) => n + s.dataRows, 0);
    if (shared) toast(`Opened ${escapeHtml(info.name)} here too: both windows stay in sync`, 2500);
    else toast(`Opened ${fmtNum(rows)} rows in ${fmtNum(Math.round(performance.now() - t0))} ms`, 2500);
  } catch (e) {
    await showError(e, "Can't open file");
  }
}

export async function cmdOpen() {
  let path: string | null;
  if (isTauri) {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const r = await open({ multiple: false, directory: false, filters: [{ name: "Spreadsheets", extensions: OPEN_EXT }] });
    path = typeof r === "string" ? r : null;
  } else {
    path = prompt("Path of the file to open (dev bridge):");
  }
  if (path) await openPath(path);
}

/**
 * The opened file holds things SpeedSheet can't keep: saving over it would remove them.
 * Ask first. Returns "copy" (save under a new name), "overwrite" or "cancel".
 */
async function confirmLosses(losses: string[]): Promise<"copy" | "overwrite" | "cancel"> {
  const msg =
    `"${app.book.name}" contains things SpeedSheet can't keep yet. Saving over it would remove:\n\n` +
    losses.map((l) => `• ${l}`).join("\n") +
    `\n\nSave a copy instead, so the original stays complete?`;
  if (isTauri) {
    const { message } = await import("@tauri-apps/plugin-dialog");
    const copy = "Save a copy…";
    const overwrite = "Overwrite";
    const r = await message(msg, {
      title: "Saving would remove content",
      kind: "warning",
      buttons: { yes: copy, no: overwrite, cancel: "Cancel" },
    });
    return r === copy || r === "Yes" ? "copy" : r === overwrite || r === "No" ? "overwrite" : "cancel";
  }
  if (confirm(msg)) return "copy";
  return confirm(`Overwrite "${app.book.name}" and remove these anyway?`) ? "overwrite" : "cancel";
}

function samePath(a: string | null, b: string | null) {
  const norm = (p: string) => p.replace(/\\/g, "/").toLowerCase();
  return a !== null && b !== null && norm(a) === norm(b);
}

export async function cmdSave(forceAs = false): Promise<boolean> {
  await commitEdit(0, 0);
  const book = app.book;
  let path: string | null = book.path;
  let force = false;
  if (!forceAs && path && book.writable && book.losses.length) {
    const choice = await confirmLosses(book.losses);
    if (choice === "cancel") return false;
    if (choice === "copy") forceAs = true;
    else force = true;
  }
  if (forceAs || !path || !book.writable) {
    const base = (book.path ?? "Untitled").replace(/\.(xls|xlsb|ods|xlsm|xlsx|csv|tsv|txt)$/i, "");
    const ext = book.format === "csv" ? "csv" : book.macros ? "xlsm" : "xlsx";
    if (isTauri) {
      const { save } = await import("@tauri-apps/plugin-dialog");
      path = await save({
        defaultPath: `${base}${forceAs && book.path ? " (copy)" : ""}.${ext}`,
        filters: [
          ...(book.macros ? [{ name: "Excel macro-enabled workbook", extensions: ["xlsm"] }] : []),
          { name: "Excel workbook", extensions: ["xlsx"] },
          { name: "CSV (current sheet)", extensions: ["csv"] },
        ],
      });
    } else {
      path = prompt("Save as (path on the dev bridge machine):", `${base}.${ext}`);
    }
    if (!path) return false;
    if (!/\.(xlsx|xlsm|csv|tsv)$/i.test(path)) path += ".xlsx";
    if (!force && book.losses.length && samePath(path, book.path)) {
      const choice = await confirmLosses(book.losses);
      if (choice === "cancel") return false;
      if (choice === "copy") return cmdSave(true);
      force = true;
    }
  }
  try {
    const info = await busy("Saving…", () => call<BookInfo>("save_file", { path, sheet: app.sheet, force }));
    applyInfo(info);
    toast(`Saved ${escapeHtml(info.name)}`, 1800);
    return true;
  } catch (e) {
    await showError(e, "Can't save");
    return false;
  }
}

export async function cmdUndo(redo = false) {
  await commitEdit(0, 0);
  const moves = grid.moveCount;
  const res = await busy(redo ? "Redoing…" : "Undoing…", () => call<EditResult | null>(redo ? "redo" : "undo"));
  // Show the undone change, unless the user moved on while it was undone (found by ui-fuzz).
  if (res) afterEdit(res, grid.moveCount === moves);
}
