// Filter popover (Excel-style value list + condition) and a small context menu.
import type { FilterCond } from "./api";

const pop = document.getElementById("popover")!;
const state = { onClose: null as (() => void) | null };

export function closePopover() {
  if (pop.hidden) return;
  pop.hidden = true;
  pop.innerHTML = "";
  const cb = state.onClose;
  state.onClose = null;
  cb?.();
}

document.addEventListener("mousedown", (e) => {
  if (!pop.hidden && !pop.contains(e.target as Node)) closePopover();
});

function place(x: number, y: number) {
  pop.hidden = false;
  const r = pop.getBoundingClientRect();
  const left = Math.max(8, Math.min(x, window.innerWidth - r.width - 8));
  const top = Math.max(8, Math.min(y, window.innerHeight - r.height - 8));
  pop.style.left = `${left}px`;
  pop.style.top = `${top}px`;
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, props: Record<string, unknown> = {}, ...kids: (Node | string)[]) {
  const e = document.createElement(tag);
  const { style, ...rest } = props;
  Object.assign(e, rest);
  if (typeof style === "string") e.style.cssText = style;
  for (const k of kids) e.append(k);
  return e;
}

export interface MenuItem {
  label?: string;
  hint?: string;
  action?: () => unknown;
  disabled?: boolean;
  separator?: boolean;
}

export function openMenu(x: number, y: number, items: MenuItem[]) {
  closePopover();
  pop.innerHTML = "";
  pop.style.width = "220px";
  pop.style.gap = "0";
  pop.style.padding = "4px";
  for (const it of items) {
    if (it.separator) {
      pop.append(el("div", { style: "height:1px;background:var(--border);margin:4px 2px" }));
      continue;
    }
    const b = el("button", { disabled: !!it.disabled }, el("span", { textContent: it.label ?? "" }));
    b.style.justifyContent = "space-between";
    b.style.width = "100%";
    if (it.hint) b.append(el("span", { textContent: it.hint, style: "color:var(--muted);font-size:12px" }));
    b.onclick = () => {
      closePopover();
      it.action?.();
    };
    pop.append(b);
  }
  state.onClose = () => {
    pop.style.width = "";
    pop.style.gap = "";
    pop.style.padding = "";
  };
  place(x, y);
}

const OPS: [string, string][] = [
  ["", "No condition"],
  ["contains", "Contains"],
  ["not_contains", "Does not contain"],
  ["equals", "Equals"],
  ["not_equals", "Does not equal"],
  ["gt", "Greater than"],
  ["ge", "Greater or equal"],
  ["lt", "Less than"],
  ["le", "Less or equal"],
  ["empty", "Is empty"],
  ["not_empty", "Is not empty"],
];

const MAX_LISTED = 1000;

export interface FilterPopoverOpts {
  x: number;
  y: number;
  col: number;
  title: string;
  header: boolean;
  current: FilterCond | null;
  loadValues: (header: boolean) => Promise<{ values: [string, number][]; truncated: boolean }>;
  onSort: (ascending: boolean, header: boolean) => void;
  onApply: (cond: FilterCond | null, header: boolean) => void;
}

export function openFilterPopover(o: FilterPopoverOpts) {
  closePopover();
  pop.innerHTML = "";
  const headerChk = el("input", { type: "checkbox", checked: o.header });
  const sortAsc = el("button", { className: "bordered", textContent: "Sort A → Z" });
  const sortDesc = el("button", { className: "bordered", textContent: "Sort Z → A" });
  sortAsc.onclick = () => {
    closePopover();
    o.onSort(true, headerChk.checked);
  };
  sortDesc.onclick = () => {
    closePopover();
    o.onSort(false, headerChk.checked);
  };

  const opSel = el("select");
  for (const [v, label] of OPS) opSel.append(el("option", { value: v, textContent: label }));
  const opVal = el("input", { type: "text", placeholder: "Value" });
  const cur = o.current;
  if (cur && cur.op !== "in" && cur.op !== "not_in") {
    opSel.value = cur.op;
    if ("value" in cur) opVal.value = cur.value;
  }
  const syncOp = () => (opVal.disabled = opSel.value === "" || opSel.value === "empty" || opSel.value === "not_empty");
  opSel.onchange = syncOp;
  syncOp();

  const search = el("input", { type: "text", placeholder: "Search values" });
  const list = el("div", { className: "values" });
  const note = el("div", { className: "note" });
  const allChk = el("input", { type: "checkbox", checked: true });
  const allRow = el("label", {}, allChk, el("span", { className: "label-text", textContent: "(Select all)" }));
  list.append(allRow, el("div", { className: "note", textContent: "Loading values…", style: "padding:4px 8px" }));

  let values: [string, number][] = [];
  let truncated = false;
  // Unchecked values are tracked so "select all" + a few exclusions stays cheap.
  const selected = new Map<string, boolean>();
  if (cur?.op === "in") {
    allChk.checked = false;
    selected.set("\u0000all", false);
    for (const v of cur.values) selected.set(v, true);
  } else if (cur?.op === "not_in") {
    for (const v of cur.values) selected.set(v, false);
  }
  const isChecked = (v: string) => selected.get(v) ?? selected.get("\u0000all") ?? true;

  const renderList = () => {
    const q = search.value.toLowerCase();
    list.innerHTML = "";
    list.append(allRow);
    let shown = 0;
    const frag = document.createDocumentFragment();
    for (const [v, n] of values) {
      if (q && !v.toLowerCase().includes(q)) continue;
      if (shown >= MAX_LISTED) break;
      const cb = el("input", { type: "checkbox", checked: isChecked(v) });
      cb.onchange = () => {
        selected.set(v, cb.checked);
        syncAll();
      };
      frag.append(el("label", { title: v || "(Blanks)" }, cb, el("span", { className: "label-text", textContent: v || "(Blanks)" }), el("span", { className: "count", textContent: n.toLocaleString() })));
      shown++;
    }
    list.append(frag);
    const total = values.filter(([v]) => !q || v.toLowerCase().includes(q)).length;
    note.textContent =
      shown < total || truncated ? `Showing ${shown.toLocaleString()} of ${total.toLocaleString()}${truncated ? "+" : ""} values; search to narrow down.` : "";
  };
  const syncAll = () => {
    allChk.checked = values.every(([v]) => isChecked(v));
  };
  allChk.onchange = () => {
    const q = search.value.toLowerCase();
    if (!q) {
      selected.clear();
      selected.set("\u0000all", allChk.checked);
    } else {
      for (const [v] of values) if (v.toLowerCase().includes(q)) selected.set(v, allChk.checked);
    }
    renderList();
  };
  search.oninput = renderList;

  const load = () =>
    o.loadValues(headerChk.checked).then((r) => {
      values = r.values;
      truncated = r.truncated;
      renderList();
      syncAll();
    });
  headerChk.onchange = load;
  load().catch(() => (note.textContent = "Could not load values."));

  const clearBtn = el("button", { className: "bordered", textContent: "Clear" });
  const cancelBtn = el("button", { className: "bordered", textContent: "Cancel" });
  const okBtn = el("button", { className: "primary", textContent: "Apply" });
  clearBtn.onclick = () => {
    closePopover();
    o.onApply(null, headerChk.checked);
  };
  cancelBtn.onclick = closePopover;
  const colIndex = o.col;
  okBtn.onclick = () => {
    const op = opSel.value;
    let cond: FilterCond | null = null;
    if (op === "empty" || op === "not_empty") cond = { col: colIndex, op } as FilterCond;
    else if (op) cond = { col: colIndex, op, value: opVal.value } as FilterCond;
    else {
      const checked = values.filter(([v]) => isChecked(v)).map(([v]) => v);
      const unchecked = values.filter(([v]) => !isChecked(v)).map(([v]) => v);
      // With "select all" as the baseline, store exclusions: that stays correct for values
      // beyond the listed ones, and is smaller in the common "hide a few" case.
      const baselineAll = selected.get("\u0000all") ?? true;
      if (unchecked.length) {
        cond = baselineAll || truncated ? { col: colIndex, op: "not_in", values: unchecked } : { col: colIndex, op: "in", values: checked };
      }
    }
    closePopover();
    o.onApply(cond, headerChk.checked);
  };

  pop.append(
    el("h3", { textContent: o.title }),
    el("div", { className: "row" }, sortAsc, sortDesc),
    el("div", { className: "row" }, opSel),
    el("div", { className: "row" }, opVal),
    search,
    list,
    note,
    el("label", { className: "chk" }, headerChk, " First row is a header"),
    el("div", { className: "actions" }, clearBtn, cancelBtn, okBtn),
  );
  [opVal, search].forEach((i) =>
    i.addEventListener("keydown", (e) => {
      e.stopPropagation();
      if (e.key === "Enter") okBtn.click();
      if (e.key === "Escape") closePopover();
    }),
  );
  place(o.x, o.y);
  search.focus();
}
