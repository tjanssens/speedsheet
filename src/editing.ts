// Editing a cell: the in-cell editor and the formula bar, Enter/Tab/arrows to commit, Escape to
// cancel, and the name box (type an address to jump there).
import { call, type EditResult } from "./api";
import { app, dom, grid } from "./app";
import { afterEdit, updateCellUI } from "./book";
import { endFormulaHelp, formulaKeys, formulaTyped, updateFormulaHelp } from "./formula-edit";
import { parseAddress } from "./grid";
import { showError, spawn } from "./ui";

export function startEdit(initial: string | null, mode: "enter" | "edit") {
  if (app.editing) return;
  const { editor, formulaInput } = dom;
  const p = grid.active;
  grid.ensureVisible(p);
  const r = grid.editorRect(p);
  const original = formulaInput.value;
  app.editing = { mode, where: "cell", original };
  editor.value = initial ?? original;
  editor.hidden = false;
  editor.style.left = `${r.x}px`;
  editor.style.top = `${r.y}px`;
  editor.style.minWidth = `${r.w + 1}px`;
  editor.style.width = `${r.w + 1}px`;
  editor.style.height = `${r.h + 1}px`;
  const es = grid.editorStyle(p);
  editor.style.font = es.font;
  editor.style.lineHeight = "1.25";
  editor.style.textAlign = es.align;
  editor.style.whiteSpace = es.wrap ? "pre-wrap" : "pre";
  editor.dataset.minH = String(r.h + 1);
  sizeEditor();
  formulaInput.value = editor.value;
  editor.focus();
  const end = editor.value.length;
  editor.setSelectionRange(end, end);
  updateFormulaHelp();
}

/** Grow the in-cell editor with its text, like Excel. */
export function sizeEditor() {
  const editor = dom.editor;
  editor.style.width = editor.style.minWidth;
  const needed = Math.min(editor.scrollWidth + 4, grid.host.clientWidth - editor.offsetLeft - 8);
  if (needed > editor.offsetWidth) editor.style.width = `${needed}px`;
  editor.style.height = "auto";
  const minH = Number(editor.dataset.minH || 23);
  editor.style.height = `${Math.max(minH, editor.scrollHeight + 2)}px`;
}

/** Store the edit (if it changed anything) and move the cursor by (dr, dc). */
export async function commitEdit(dr: number, dc: number, refocus = true) {
  if (!app.editing) return;
  const { where, original } = app.editing;
  const value = where === "cell" ? dom.editor.value : dom.formulaInput.value;
  app.editing = null;
  dom.editor.hidden = true;
  endFormulaHelp();
  const { row, col } = grid.active;
  if (refocus) grid.scroller.focus({ preventScroll: true });
  if (dr || dc) grid.advance(dr, dc);
  if (value === original) {
    updateCellUI();
    return;
  }
  try {
    const res = await call<EditResult>("set_inputs", { sheet: app.sheet, row, col, grid: [[value]] });
    afterEdit(res, false);
  } catch (e) {
    await showError(e);
  }
}

export function cancelEdit() {
  if (!app.editing) return;
  dom.formulaInput.value = app.editing.original;
  app.editing = null;
  dom.editor.hidden = true;
  endFormulaHelp();
  grid.scroller.focus({ preventScroll: true });
}

const ARROWS: Record<string, [number, number]> = { ArrowUp: [-1, 0], ArrowDown: [1, 0], ArrowLeft: [0, -1], ArrowRight: [0, 1] };

function editorKey(e: KeyboardEvent) {
  const editing = app.editing;
  if (!editing) return;
  const editor = dom.editor;
  const k = e.key;
  if (k === "Enter" && !e.altKey) {
    e.preventDefault();
    spawn(commitEdit(e.shiftKey ? -1 : 1, 0));
  } else if (k === "Enter" && e.altKey) {
    e.preventDefault();
    editor.setRangeText("\n", editor.selectionStart, editor.selectionEnd, "end");
    editor.dispatchEvent(new Event("input"));
  } else if (k === "Tab") {
    e.preventDefault();
    spawn(commitEdit(0, e.shiftKey ? -1 : 1));
  } else if (k === "Escape") {
    e.preventDefault();
    cancelEdit();
  } else if (editing.mode === "enter" && k in ARROWS) {
    e.preventDefault();
    const [dr, dc] = ARROWS[k];
    spawn(commitEdit(dr, dc));
  } else if (k === "F2") {
    editing.mode = editing.mode === "enter" ? "edit" : "enter";
  }
  e.stopPropagation();
}

export function setupEditing() {
  const { editor, formulaInput, nameBox } = dom;
  editor.addEventListener("input", () => {
    formulaInput.value = editor.value;
    sizeEditor();
    formulaTyped();
  });
  formulaInput.addEventListener("input", () => {
    if (app.editing?.where === "bar") formulaTyped();
  });
  for (const el of [editor, formulaInput]) {
    el.addEventListener("click", () => updateFormulaHelp());
    el.addEventListener("keyup", (ev) => {
      const e = ev as KeyboardEvent;
      if (e.key.startsWith("Arrow") || e.key === "Home" || e.key === "End") updateFormulaHelp();
    });
    el.addEventListener("keydown", (e) => formulaKeys(e as KeyboardEvent), true);
  }
  editor.addEventListener("keydown", editorKey);

  formulaInput.addEventListener("focus", () => {
    if (app.editing) return;
    app.editing = { mode: "edit", where: "bar", original: formulaInput.value };
    // The caret is placed after focus; the click and keyup handlers above update the help then.
    updateFormulaHelp();
  });
  formulaInput.addEventListener("blur", () => {
    if (app.editing?.where === "bar") spawn(commitEdit(0, 0, false));
  });
  formulaInput.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      spawn(commitEdit(1, 0));
    } else if (e.key === "Escape") {
      e.preventDefault();
      cancelEdit();
    } else if (e.key === "Tab") {
      e.preventDefault();
      spawn(commitEdit(0, e.shiftKey ? -1 : 1));
    }
  });

  nameBox.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      const p = parseAddress(nameBox.value);
      if (p) {
        grid.scroller.focus();
        grid.select(p);
      } else updateCellUI(true);
    } else if (e.key === "Escape") {
      updateCellUI(true);
      grid.scroller.focus();
    }
  });
  nameBox.addEventListener("focus", () => nameBox.select());
  nameBox.addEventListener("blur", () => updateCellUI());

  grid.onEditRequest = () => startEdit(null, "edit");
}
