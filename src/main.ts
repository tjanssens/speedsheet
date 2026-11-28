// Start-up: wire the modules together and load the workbook this window shows.
//
// Where things live: app.ts (window state), book.ts (tabs, title, status), editing.ts and
// formula-edit.ts (typing in cells), clipboard.ts, files.ts (new/open/save/undo), sort-filter.ts,
// commands.ts (toolbar, menus, find, print, theme), keyboard.ts, peers.ts (other windows),
// grid*.ts (the canvas grid). docs/features.md maps each feature to its code and tests.
import { call, closeBrowserView, errorText, isTauri, native, onBackendEvent, type AppInfo, type BookInfo, type DocChanged, type Peers } from "./api";
import { app, grid } from "./app";
import { applyInfo, updateCellUI } from "./book";
import { applyTheme, setupCommands } from "./commands";
import { commitEdit, setupEditing } from "./editing";
import { confirmDiscard, openPath } from "./files";
import { setupKeyboard } from "./keyboard";
import { applyPeers, refreshPeers, remoteChange, sendPresence } from "./peers";
import { after } from "./schedule";
import { checkForUpdates } from "./updates";
import { escapeHtml, showError, spawn } from "./ui";
import { exposeTestHooks } from "./test-hooks";

// Safety net: an async action that fails without its own error handling must never fail silently.
window.addEventListener("unhandledrejection", (ev) => {
  ev.preventDefault();
  console.error(ev.reason);
  spawn(showError(ev.reason));
});

grid.onSelectionChange = () => {
  if (app.editing?.where === "cell") spawn(commitEdit(0, 0));
  updateCellUI();
  sendPresence();
};

setupEditing();
setupCommands();
setupKeyboard();
applyTheme();

async function desktopIntegration() {
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  const { getCurrentWebview } = await import("@tauri-apps/api/webview");
  const { getCurrentWebviewWindow } = await import("@tauri-apps/api/webviewWindow");
  const win = getCurrentWindow();
  await win.onCloseRequested(async (ev) => {
    if (!app.book.modified || app.peers.count > 1) return;
    ev.preventDefault();
    if (await confirmDiscard()) await win.destroy();
  });
  await getCurrentWebview().onDragDropEvent((ev) => {
    if (ev.payload.type === "drop" && ev.payload.paths.length) spawn(openPath(ev.payload.paths[0]));
  });
  await getCurrentWebviewWindow().listen<string>("open-file", (ev) => {
    spawn(native("take_pending_open").then(() => openPath(ev.payload)));
  });
  const pending = await native<string | null>("take_pending_open");
  if (pending) await openPath(pending);
}

async function init() {
  try {
    app.book = await call<BookInfo>("book_info");
  } catch (e) {
    document.body.innerHTML = `<p style="padding:24px">Can't reach the SpeedSheet engine: ${escapeHtml(errorText(e))}.<br>
      In a browser, start the dev bridge: <code>cargo run -p speedsheet --example dev_bridge</code></p>`;
    return;
  }
  applyInfo(app.book, 0);
  grid.select({ row: 0, col: 0 });
  grid.scroller.focus();
  exposeTestHooks();
  await onBackendEvent<DocChanged>("doc-changed", remoteChange);
  await onBackendEvent<Peers>("peers", applyPeers);
  await refreshPeers();
  app.appInfo = (await native<AppInfo>("app_info").catch(() => undefined)) ?? null;
  // Quietly, once the window is up: an update check must never slow down opening a file.
  after(4000, () => spawn(checkForUpdates(app.appInfo, false)));
  if (isTauri) await desktopIntegration();
}

window.addEventListener("beforeunload", (e) => {
  if (!isTauri && app.book?.modified && app.peers.count <= 1) e.preventDefault();
});
window.addEventListener("pagehide", closeBrowserView);

spawn(init());
