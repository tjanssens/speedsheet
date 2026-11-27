// Formula editing help, like Excel: function autocomplete, argument hints, reference colors,
// and "point mode" (clicking a cell while typing a formula inserts its address).
import { BY_NAME, FUNCTIONS, type FnInfo } from "./functions";
import { colName, type Rect } from "./grid";

export const REF_COLORS = ["#1f6fd1", "#d13f1f", "#7d3fc4", "#1d8a4a", "#c77700", "#c0307a", "#00838f", "#6d4c41"];

const IDENT = /[A-Za-z0-9_.]/;
const POINT_BEFORE = "=(,;+-*/^&<>:";

export interface Analysis {
  /** Identifier being typed at the caret (a function name candidate). */
  token: { start: number; end: number; text: string } | null;
  /** Innermost function call around the caret and which argument the caret is in. */
  call: { fn: FnInfo; arg: number } | null;
  /** A cell reference can be inserted at the caret. */
  insertable: boolean;
}

export function analyze(text: string, caret: number): Analysis | null {
  if (!text.startsWith("=")) return null;
  const stack: { name: string; arg: number }[] = [];
  let inStr = false;
  for (let i = 1; i < caret; i++) {
    const ch = text[i];
    if (ch === '"') inStr = !inStr;
    if (inStr) continue;
    if (ch === "(") {
      let j = i;
      while (j > 1 && IDENT.test(text[j - 1])) j--;
      stack.push({ name: text.slice(j, i).toUpperCase(), arg: 0 });
    } else if (ch === ")") stack.pop();
    else if ((ch === "," || ch === ";") && stack.length) stack[stack.length - 1].arg++;
  }
  if (inStr) return { token: null, call: null, insertable: false };
  let start = caret;
  while (start > 1 && IDENT.test(text[start - 1])) start--;
  let token: Analysis["token"] = null;
  const word = text.slice(start, caret);
  const before = text.slice(0, start).trimEnd().slice(-1);
  if (word && /^[A-Za-z_]/.test(word) && POINT_BEFORE.includes(before) && before !== "") {
    token = { start, end: caret, text: word };
  }
  const prev = text.slice(0, caret).trimEnd().slice(-1);
  const insertable = POINT_BEFORE.includes(prev) && prev !== "";
  let call: Analysis["call"] = null;
  for (let k = stack.length - 1; k >= 0; k--) {
    const fn = BY_NAME.get(stack[k].name);
    if (fn) {
      call = { fn, arg: stack[k].arg };
      break;
    }
  }
  return { token, call, insertable };
}

export function suggestions(prefix: string): FnInfo[] {
  const p = prefix.toUpperCase();
  return FUNCTIONS.filter((f) => f.name.startsWith(p)).slice(0, 12);
}

/** "A1" or "A1:C3" for a rectangle (data rows/cols). */
export function refText(r: Rect): string {
  const a = `${colName(r.c0)}${r.r0 + 1}`;
  if (r.r0 === r.r1 && r.c0 === r.c1) return a;
  return `${a}:${colName(r.c1)}${r.r1 + 1}`;
}

const REF = /(\$?[A-Za-z]{1,3}\$?\d{1,7})(?::(\$?[A-Za-z]{1,3}\$?\d{1,7}))?/y;

function parseCell(s: string): { row: number; col: number } | null {
  const m = /^\$?([A-Za-z]{1,3})\$?(\d+)$/.exec(s);
  if (!m) return null;
  let col = 0;
  for (const ch of m[1].toUpperCase()) col = col * 26 + (ch.charCodeAt(0) - 64);
  return { row: parseInt(m[2], 10) - 1, col: col - 1 };
}

/** Same-sheet references in a formula, with their position in the text and a color each. */
export function findRefs(text: string): { start: number; end: number; rect: Rect; color: string }[] {
  const out: { start: number; end: number; rect: Rect; color: string }[] = [];
  if (!text.startsWith("=")) return out;
  let inStr = false;
  const seen = new Map<string, string>();
  for (let i = 1; i < text.length; i++) {
    const ch = text[i];
    if (ch === '"') {
      inStr = !inStr;
      continue;
    }
    if (inStr) continue;
    const prev = text[i - 1];
    if (IDENT.test(prev) || prev === "!" || prev === "'") continue;
    REF.lastIndex = i;
    const m = REF.exec(text);
    if (!m) continue;
    const end = i + m[0].length;
    if (end < text.length && (IDENT.test(text[end]) || text[end] === "(" || text[end] === "!")) continue;
    const a = parseCell(m[1]);
    const b = m[2] ? parseCell(m[2]) : a;
    if (!a || !b) continue;
    const rect = { r0: Math.min(a.row, b.row), c0: Math.min(a.col, b.col), r1: Math.max(a.row, b.row), c1: Math.max(a.col, b.col) };
    const key = `${rect.r0}:${rect.c0}:${rect.r1}:${rect.c1}`;
    let color = seen.get(key);
    if (!color) {
      color = REF_COLORS[seen.size % REF_COLORS.length];
      seen.set(key, color);
    }
    out.push({ start: i, end, rect, color });
    i = end - 1;
  }
  return out;
}

/** The argument hint as HTML: the signature with the current argument in bold. */
export function hintHtml(fn: FnInfo, arg: number): string {
  const esc = (s: string) => s.replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" })[c]!);
  const repeating = fn.args.length > 0 && fn.args[fn.args.length - 1] === "…";
  const n = fn.args.length;
  // Past the listed arguments of a repeating function, keep the last real one bold.
  const at = repeating && arg >= n - 1 ? n - 2 : arg;
  const parts = fn.args.map((a, i) => (i === at ? `<b>${esc(a)}</b>` : esc(a)));
  return `<span class="sig">${fn.name}(${parts.join(", ")})</span><span class="desc">${esc(fn.desc)}</span>`;
}
