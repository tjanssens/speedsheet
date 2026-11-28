// Thin client for the Rust side. In the desktop app every call goes through one Tauri command;
// in a plain browser (development) it goes to the dev bridge on 127.0.0.1:1421.
import { invoke } from "@tauri-apps/api/core";
import { after } from "./schedule";

export const isTauri = "__TAURI_INTERNALS__" in window;
const BRIDGE = "http://127.0.0.1:1421";
/** In the browser every tab is a window of its own; the desktop app knows its windows itself. */
const VIEW = Math.random().toString(36).slice(2, 10);

/** Engine calls that have not answered yet (the UI tests wait until it is 0). */
const calls = { inFlight: 0, pending: new Map<string, number>(), lastError: "" };
export const callsInFlight = () => calls.inFlight;
/** For test failure messages: which calls are still open, and the last call that failed. */
export const callsDebug = () => ({ pending: Object.fromEntries(calls.pending), lastError: calls.lastError });

export async function call<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  // The dev bridge's event poll runs all the time; it is not work the UI waits for.
  const counted = cmd !== "_events";
  if (counted) {
    calls.inFlight++;
    calls.pending.set(cmd, (calls.pending.get(cmd) ?? 0) + 1);
  }
  try {
    if (isTauri) return await invoke<T>("call", { cmd, args });
    const res = await fetch(`${BRIDGE}/${cmd}?view=${VIEW}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(args),
    });
    const body: unknown = await res.json();
    if (!res.ok) throw body;
    return body as T;
  } catch (e) {
    calls.lastError = `${cmd}: ${e instanceof Error ? e.message : JSON.stringify(e)}`;
    throw e;
  } finally {
    if (counted) {
      calls.inFlight--;
      const n = (calls.pending.get(cmd) ?? 1) - 1;
      if (n) calls.pending.set(cmd, n);
      else calls.pending.delete(cmd);
    }
  }
}

/** Listen for messages from the engine meant for this window: `doc-changed` when another window
 * on the same file changed it, `peers` when the other windows on it or their cursors changed. */
export async function onBackendEvent<T>(name: string, handler: (payload: T) => void) {
  if (isTauri) {
    const { getCurrentWebviewWindow } = await import("@tauri-apps/api/webviewWindow");
    await getCurrentWebviewWindow().listen<T>(name, (e) => handler(e.payload));
    return;
  }
  (events.handlers[name] ??= []).push(handler as (p: unknown) => void);
  if (!events.polling) {
    events.polling = true;
    await pollEvents();
  }
}

const events = { handlers: {} as Record<string, ((p: unknown) => void)[]>, polling: false };

/** The dev bridge has no push channel: fetch this tab's messages a few times per second. */
async function pollEvents() {
  try {
    const evs = await call<{ event: string; payload: unknown }[]>("_events");
    for (const ev of evs) for (const h of events.handlers[ev.event] ?? []) h(ev.payload);
  } catch {
    /* bridge restarting */
  }
  // pollEvents handles its own errors, so the catch only satisfies the rule that none get lost.
  after(evsDelay(), () => {
    pollEvents().catch(console.error);
  });
}
const evsDelay = () => (document.hidden ? 1000 : 150);

/** Tell the dev bridge this tab is gone (the desktop app sees its windows close). */
export function closeBrowserView() {
  if (!isTauri) navigator.sendBeacon(`${BRIDGE}/close_view?view=${VIEW}`, "null");
}

/** Desktop-only commands (printing, app info, feedback); undefined in the browser. */
export async function native<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T | undefined> {
  if (!isTauri) return undefined;
  return invoke<T>(cmd, args);
}

export type FilterOp =
  | { op: "contains" | "not_contains" | "equals" | "not_equals" | "gt" | "lt" | "ge" | "le"; value: string }
  | { op: "empty" | "not_empty" }
  | { op: "in" | "not_in"; values: string[] };

export type FilterCond = FilterOp & { col: number };

export interface Filter {
  header: boolean;
  conds: FilterCond[];
}

export interface SheetInfo {
  name: string;
  rows: number;
  dataRows: number;
  cols: number;
  filter: Filter | null;
  header: boolean;
  colWidths: (number | null)[];
}

export interface BookInfo {
  path: string | null;
  name: string;
  format: "xlsx" | "xls" | "xlsb" | "ods" | "csv" | "new";
  writable: boolean;
  modified: boolean;
  /** What saving over the opened file would remove (charts, data validation, ...). */
  losses: string[];
  /** The workbook has macros, so it can be saved as .xlsm. */
  macros: boolean;
  canUndo: boolean;
  canRedo: boolean;
  sheets: SheetInfo[];
}

export interface Block {
  rows: number[];
  cols: number;
  text: string[];
  kinds: number[];
  /** Style id per cell (index into Styles.styles). */
  styles: number[];
  /** Conditional format per cell: 0 none, else dxf index + 1. */
  cf?: number[];
}

export type LineStyle =
  | "thin" | "medium" | "thick" | "double" | "hair" | "dashed" | "dotted" | "mediumDashed"
  | "dashDot" | "mediumDashDot" | "dashDotDot" | "mediumDashDotDot" | "slantDashDot";

export interface BorderLine {
  style: LineStyle;
  color: number;
}

export interface CellStyle {
  bold?: boolean;
  italic?: boolean;
  underline?: boolean;
  strike?: boolean;
  fontName?: string;
  fontSize?: number;
  color?: number;
  fill?: number;
  hAlign: "general" | "left" | "center" | "right" | "justify" | "fill" | "centeracross" | "distributed";
  vAlign: "bottom" | "center" | "top" | "justify" | "distributed";
  wrap?: boolean;
  indent?: number;
  numFmt?: string;
  /** left, right, top, bottom */
  borders: (BorderLine | null)[];
}

export interface Styles {
  styles: CellStyle[];
  dxfs: CellStyle[];
}

export interface Area {
  r0: number;
  c0: number;
  r1: number;
  c1: number;
}

export interface Layout {
  /** Runs of [first view row, count, height px]. */
  rowHeights: [number, number, number][];
  defaultRowHeight: number | null;
  defaultColWidth: number | null;
  merges: Area[];
  /** [rows, cols] frozen at the top/left. */
  freeze: [number, number] | null;
  autofilter: { row: number; dataRow: number; c0: number; c1: number } | null;
  hideGrid: boolean;
}

export interface Presence {
  sheet: number;
  row: number;
  col: number;
  r0: number;
  c0: number;
  r1: number;
  c1: number;
}

/** The other windows on this window's file. */
export interface Peers {
  /** This window's 1-based number (the ":2" in "Book.xlsx:2"). */
  number: number;
  /** Windows on the file, this one included. */
  count: number;
  peers: { view: string; number: number; presence: Presence | null }[];
}

export interface DocChanged {
  by: string;
  cmd: string;
  info: BookInfo;
}

export interface EditResult {
  info: BookInfo;
  sheet: number;
  row: number | null;
  col: number;
  recalculated: number;
}

export interface Match {
  sheet: number;
  row: number;
  view_row: number;
  col: number;
}

export interface Stats {
  count: number;
  numbers: number;
  sum: number;
  min: number | null;
  max: number | null;
}

export interface Copied {
  tsv: string;
  html: string;
  inputs: string[][];
}

export interface AppInfo {
  version: string;
  os: string;
  osVersion: string;
  arch: string;
  feedbackConfigured: boolean;
  updatesConfigured: boolean;
  repo: string;
}

export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return JSON.stringify(e);
}
