// Help while typing a formula: references in color, clicking or arrowing to cells to insert a
// reference (point mode), function autocomplete and the argument hint.
import { app, dom, grid } from "./app";
import { sizeEditor } from "./editing";
import { analyze, findRefs, hintHtml, refText, suggestions } from "./formulaHelp";
import type { FnInfo } from "./functions";
import type { Pos, Rect } from "./grid";

function popup(id: string) {
  const el = document.createElement("div");
  el.id = id;
  el.hidden = true;
  document.body.append(el);
  return el;
}

const fxPop = popup("fx-pop");
const fxHint = popup("fx-hint");

const fx = {
  /** The reference most recently inserted by pointing, so the next pick replaces it. */
  pointRef: null as { start: number; end: number; anchor: Pos; cursor: Pos } | null,
  list: [] as FnInfo[],
  index: 0,
};

function editInput(): HTMLInputElement | HTMLTextAreaElement {
  return app.editing?.where === "bar" ? dom.formulaInput : dom.editor;
}

function setEditText(text: string, caret: number) {
  const el = editInput();
  el.value = text;
  el.setSelectionRange(caret, caret);
  if (el === dom.editor) {
    dom.formulaInput.value = text;
    sizeEditor();
  } else dom.editor.value = text;
}

function placeBelow(div: HTMLElement, el: HTMLElement, offsetY = 0) {
  const r = el.getBoundingClientRect();
  div.style.left = `${Math.max(4, Math.min(r.left, window.innerWidth - 380))}px`;
  div.style.top = `${r.bottom + 2 + offsetY}px`;
}

/** The user typed: a following pick inserts a new reference instead of replacing the last one. */
export function formulaTyped() {
  fx.pointRef = null;
  updateFormulaHelp();
}

export function updateFormulaHelp() {
  if (!app.editing) return endFormulaHelp();
  const el = editInput();
  const text = el.value;
  const caret = el.selectionStart ?? text.length;
  const a = analyze(text, caret);
  // Colored references in the grid.
  grid.refHighlights = a && !grid.filtered ? findRefs(text).map((r) => ({ rect: r.rect, color: r.color })) : [];
  // Point mode: clicking a cell inserts its reference.
  const atPointRef = fx.pointRef !== null && caret === fx.pointRef.end;
  grid.pointMode = a && (a.insertable || atPointRef) ? { onPick: (r, dragging) => pickRef(r, dragging) } : null;
  // Function autocomplete.
  fx.list = a?.token ? suggestions(a.token.text) : [];
  if (fx.list.length === 1 && fx.list[0].name === a?.token?.text.toUpperCase()) fx.list = [];
  if (fx.list.length) {
    fx.index = Math.min(fx.index, fx.list.length - 1);
    fxPop.innerHTML = "";
    fx.list.forEach((f, i) => {
      const item = document.createElement("div");
      item.className = "item" + (i === fx.index ? " sel" : "");
      item.innerHTML = `<b>${f.name}</b><span>${f.desc}</span>`;
      item.onmousedown = (e) => {
        e.preventDefault();
        fx.index = i;
        acceptSuggestion();
      };
      fxPop.append(item);
    });
    placeBelow(fxPop, el);
    fxPop.hidden = false;
  } else {
    fxPop.hidden = true;
    fx.index = 0;
  }
  // Argument hint.
  if (a?.call && fx.list.length === 0) {
    fxHint.innerHTML = hintHtml(a.call.fn, a.call.arg);
    placeBelow(fxHint, el);
    fxHint.hidden = false;
  } else fxHint.hidden = true;
  grid.render();
}

export function endFormulaHelp() {
  fxPop.hidden = true;
  fxHint.hidden = true;
  fx.list = [];
  fx.pointRef = null;
  grid.pointMode = null;
  if (grid.refHighlights.length) {
    grid.refHighlights = [];
    grid.render();
  }
}

function acceptSuggestion() {
  const el = editInput();
  const text = el.value;
  const caret = el.selectionStart ?? text.length;
  const a = analyze(text, caret);
  const f = fx.list[fx.index];
  if (!a?.token || !f) return;
  let end = a.token.end;
  while (end < text.length && /[A-Za-z0-9_.]/.test(text[end])) end++;
  const hasParen = text[end] === "(";
  const insert = f.name + (hasParen ? "" : "(");
  const next = text.slice(0, a.token.start) + insert + text.slice(end);
  setEditText(next, a.token.start + insert.length + (hasParen ? 1 : 0));
  fx.index = 0;
  updateFormulaHelp();
}

/** Insert (or replace the last pointed) reference at the caret. */
function pickRef(r: Rect, dragging: boolean) {
  const el = editInput();
  const text = el.value;
  const caret = el.selectionStart ?? text.length;
  const ref = refText(r);
  const last = fx.pointRef;
  const [start, end] = last && caret === last.end ? [last.start, last.end] : [caret, caret];
  const next = text.slice(0, start) + ref + text.slice(end);
  setEditText(next, start + ref.length);
  const anchor = dragging && last ? last.anchor : { row: r.r0, col: r.c0 };
  fx.pointRef = { start, end: start + ref.length, anchor, cursor: { row: r.r1, col: r.c1 } };
  el.focus();
  updateFormulaHelp();
}

const ARROWS: Record<string, [number, number]> = { ArrowUp: [-1, 0], ArrowDown: [1, 0], ArrowLeft: [0, -1], ArrowRight: [0, 1] };

/** Keys while editing: autocomplete navigation, and arrow keys pointing at cells in a formula. */
export function formulaKeys(e: KeyboardEvent) {
  const editing = app.editing;
  if (!editing) return;
  const stop = () => {
    e.preventDefault();
    e.stopImmediatePropagation();
  };
  if (!fxPop.hidden && fx.list.length) {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      stop();
      fx.index = (fx.index + (e.key === "ArrowDown" ? 1 : -1) + fx.list.length) % fx.list.length;
      updateFormulaHelp();
      return;
    }
    if (e.key === "Tab" || (e.key === "Enter" && !e.altKey)) {
      stop();
      acceptSuggestion();
      return;
    }
    if (e.key === "Escape") {
      stop();
      fxPop.hidden = true;
      fx.list = [];
      return;
    }
  }
  if (e.key in ARROWS && editing.mode === "enter" && grid.pointMode && !grid.filtered) {
    stop();
    const [dr, dc] = ARROWS[e.key];
    const base = fx.pointRef?.cursor ?? { row: grid.active.row, col: grid.active.col };
    const cur = { row: Math.max(0, base.row + dr), col: Math.max(0, base.col + dc) };
    const anchor = e.shiftKey && fx.pointRef ? fx.pointRef.anchor : cur;
    const rect = {
      r0: Math.min(anchor.row, cur.row),
      c0: Math.min(anchor.col, cur.col),
      r1: Math.max(anchor.row, cur.row),
      c1: Math.max(anchor.col, cur.col),
    };
    pickRef(rect, e.shiftKey);
    if (fx.pointRef) {
      fx.pointRef.anchor = anchor;
      fx.pointRef.cursor = cur;
    }
    grid.ensureVisible(cur);
  }
}
