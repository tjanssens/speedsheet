// Small UI helpers shared by every module: DOM lookup, toasts, the busy overlay, dialogs, and
// `spawn`, the one way to start async work without awaiting it (its failure reaches showError).
import { errorText, isTauri } from "./api";
import { after } from "./schedule";

export const $ = <T extends HTMLElement = HTMLElement>(sel: string) => document.querySelector(sel) as T;

export const isMac = navigator.platform.toUpperCase().includes("MAC");
export const modKey = (e: KeyboardEvent | MouseEvent) => (isMac ? e.metaKey : e.ctrlKey);

const toastState = { cancel: () => {} };

export function toast(html: string, ms = 3500) {
  const t = $("#toast");
  t.innerHTML = html;
  t.hidden = false;
  toastState.cancel();
  toastState.cancel = after(ms, () => (t.hidden = true));
}

export function escapeHtml(s: string) {
  return s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]!);
}

export function fmtNum(n: number) {
  return n.toLocaleString(undefined, { maximumFractionDigits: 6 });
}

/** Show "text" over the app while `fn` runs, unless it finishes within 120 ms. */
export async function busy<T>(text: string, fn: () => Promise<T>): Promise<T> {
  const el = $("#busy");
  const cancel = after(120, () => {
    $("#busy-text").textContent = text;
    el.hidden = false;
  });
  try {
    return await fn();
  } finally {
    cancel();
    el.hidden = true;
  }
}

export async function showError(e: unknown, title = "Something went wrong") {
  const msg = errorText(e);
  if (isTauri) {
    const { message } = await import("@tauri-apps/plugin-dialog");
    await message(msg, { title, kind: "error" });
  } else {
    alert(`${title}\n\n${msg}`);
  }
}

/** Start async work without waiting for it. A failure is shown to the user, never lost. */
export function spawn(work: Promise<unknown>, title?: string) {
  work.catch((e: unknown) => {
    console.error(e);
    showError(e, title).catch(console.error);
  });
}

export async function confirmDialog(msg: string, title: string, okLabel = "OK"): Promise<boolean> {
  if (isTauri) {
    const { ask } = await import("@tauri-apps/plugin-dialog");
    return ask(msg, { title, kind: "warning", okLabel, cancelLabel: "Cancel" });
  }
  return confirm(msg);
}
