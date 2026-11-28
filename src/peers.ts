// Other windows on the same file: their cursors, live changes they make, and the "already open"
// question when this window opens a file another window has.
import { call, type DocChanged, type Peers } from "./api";
import { app, grid } from "./app";
import { applyInfo, loadLayout, updateTitle } from "./book";
import { Debounced } from "./schedule";
import { $, spawn } from "./ui";

const PEER_COLORS = ["#e8710a", "#9334e6", "#188038", "#d93025", "#e52592", "#12a4af"];

export function applyPeers(p: Peers) {
  const was = app.peers.count;
  app.peers = p;
  grid.peerCursors = p.peers.flatMap((x) =>
    x.presence
      ? [
          {
            sheet: x.presence.sheet,
            active: { row: x.presence.row, col: x.presence.col },
            rect: { r0: x.presence.r0, c0: x.presence.c0, r1: x.presence.r1, c1: x.presence.c1 },
            color: PEER_COLORS[(x.number - 1) % PEER_COLORS.length],
            label: `Window ${x.number}`,
          },
        ]
      : [],
  );
  grid.render();
  if (was !== p.count) spawn(updateTitle());
}

export async function refreshPeers() {
  try {
    applyPeers(await call<Peers>("peers"));
  } catch {
    /* single window */
  }
  sendPresence();
}

/** Cursor moves come in bursts (arrow keys held down): tell the other windows where it rests. */
const presence = new Debounced(30, () => {
  if (!app.book) return;
  const a = grid.active;
  const r = grid.selection;
  call("set_presence", { presence: { sheet: app.sheet, row: a.row, col: a.col, r0: r.r0, c0: r.c0, r1: r.r1, c1: r.c1 } }).catch(() => {});
});

/** Tell the other windows on this file where this window's cursor is. */
export function sendPresence() {
  presence.run();
}

/** Another window on this file changed it: show the new state, keep our own place. */
export function remoteChange(ch: DocChanged) {
  const oldName = app.book.sheets[app.sheet]?.name;
  let idx = ch.info.sheets.findIndex((s) => s.name === oldName);
  if (idx < 0) idx = ch.info.sheets.length === app.book.sheets.length ? app.sheet : Math.min(app.sheet, ch.info.sheets.length - 1);
  const moved = idx !== app.sheet;
  if (ch.cmd === "set_filter" || ch.cmd === "sort") grid.dropCache();
  applyInfo(ch.info, idx);
  if (moved) sendPresence();
  spawn(loadLayout(true));
  grid.invalidate();
}

export interface OpenElsewhere {
  view: string;
  name: string;
}

/** The file is open in another window: switch there, or open it here as well (kept in sync). */
export function askAlreadyOpen(other: OpenElsewhere): Promise<"switch" | "here" | "cancel"> {
  const backdrop = $("#modal-backdrop");
  const modal = $("#modal");
  return new Promise((resolve) => {
    modal.innerHTML = "";
    const h = document.createElement("h2");
    h.textContent = "This file is already open";
    const p = document.createElement("p");
    p.textContent = `"${other.name}" is already open in another SpeedSheet window.`;
    const p2 = document.createElement("p");
    p2.textContent =
      "If you open it here too, both windows stay in sync: edits show up in both, you see where the other window is, and saving from either window keeps all changes.";
    const actions = document.createElement("div");
    actions.className = "actions";
    const mk = (label: string, cls: string, value: "switch" | "here" | "cancel") => {
      const b = document.createElement("button");
      b.className = cls;
      b.textContent = label;
      b.dataset.choice = value;
      b.onclick = () => done(value);
      return b;
    };
    const cancel = mk("Cancel", "bordered", "cancel");
    const here = mk("Open here too", "bordered", "here");
    const go = mk("Go to that window", "primary", "switch");
    actions.append(cancel, here, go);
    modal.append(h, p, p2, actions);
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        done("cancel");
      }
    };
    const done = (v: "switch" | "here" | "cancel") => {
      document.removeEventListener("keydown", onKey, true);
      backdrop.hidden = true;
      modal.innerHTML = "";
      grid.scroller.focus();
      resolve(v);
    };
    document.addEventListener("keydown", onKey, true);
    backdrop.hidden = false;
    go.focus();
  });
}

/** Whether this window shows nothing worth keeping (a fresh window opened for a file). */
export function isBlankWindow() {
  return !app.book.path && !app.book.modified && app.peers.count <= 1;
}
