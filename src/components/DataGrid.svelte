<script lang="ts" module>
  export type RowKey = { kind: "row" | "new"; index: number };
  export type Pending = string | null;
</script>

<script lang="ts">
  import { tick, untrack } from "svelte";
  import { display, editText, isBinary } from "../lib/cells";
  import type { Cell, ColumnMeta } from "../lib/types";

  let {
    columns,
    rows,
    inserted = [],
    edits = {},
    deleted = {},
    editable = false,
    rowOffset = 0,
    sort = null,
    onsort,
    onedit,
    ondelete,
    selectedRows = $bindable([]),
  }: {
    columns: ColumnMeta[];
    rows: Cell[][];
    inserted?: Record<string, Pending>[];
    edits?: Record<number, Record<string, Pending>>;
    deleted?: Record<number, boolean>;
    editable?: boolean;
    rowOffset?: number;
    sort?: { column: string; desc: boolean } | null;
    onsort?: (column: string) => void;
    onedit?: (row: RowKey, column: string, value: Pending) => void;
    ondelete?: (rows: RowKey[]) => void;
    selectedRows?: RowKey[];
  } = $props();

  const ROW_H = 24;
  const HEAD_H = 26;
  const GUTTER = 52;

  let scroller: HTMLDivElement | undefined = $state();
  let scrollTop = $state(0);
  let viewH = $state(400);
  let widths = $state<number[]>([]);

  let cur = $state<{ row: number; col: number } | null>(null);
  let anchorRow = $state<number | null>(null);
  let rowSel = $state<{ from: number; to: number } | null>(null);
  let editing = $state<{ row: number; col: number; text: string } | null>(null);
  let menu = $state<{ x: number; y: number } | null>(null);

  const total = $derived(rows.length + inserted.length);
  const first = $derived(Math.max(0, Math.floor(scrollTop / ROW_H) - 5));
  const last = $derived(Math.min(total, Math.ceil((scrollTop + viewH) / ROW_H) + 5));
  const visible = $derived(Array.from({ length: Math.max(0, last - first) }, (_, i) => first + i));
  const fullWidth = $derived(GUTTER + widths.reduce((a, b) => a + b, 0));

  // Size columns from the header and a sample of values.
  $effect(() => {
    const step = Math.max(1, Math.floor(rows.length / 200));
    const sample = rows.filter((_, i) => i % step === 0);
    widths = columns.map((c, i) => {
      // Header shows the name in bold plus the type.
      let len = c.name.length + c.type_name.length * 0.75 + 3;
      for (const r of sample) len = Math.max(len, Math.min(display(r[i]).length, 40));
      return Math.max(70, Math.min(340, len * 7.4 + 20));
    });
  });

  // New data: keep the selection only if it still points at something.
  $effect(() => {
    void rows;
    untrack(() => {
      if (cur && cur.row >= total) cur = null;
      editing = null;
    });
  });

  $effect(() => {
    const keys: RowKey[] = [];
    if (rowSel) {
      const [a, b] = rowSel.from <= rowSel.to ? [rowSel.from, rowSel.to] : [rowSel.to, rowSel.from];
      for (let r = a; r <= Math.min(b, total - 1); r++) keys.push(keyOf(r));
    } else if (cur) keys.push(keyOf(cur.row));
    selectedRows = keys;
  });

  function keyOf(r: number): RowKey {
    return r < rows.length ? { kind: "row", index: r } : { kind: "new", index: r - rows.length };
  }

  /** What the cell shows: pending edit, inserted value, or the original. */
  function valueAt(r: number, c: number): { text: string; cls: string; raw: Cell | Pending | undefined } {
    const name = columns[c].name;
    if (r >= rows.length) {
      const v = inserted[r - rows.length]?.[name];
      if (v === undefined) return { text: "DEFAULT", cls: "null", raw: undefined };
      return v === null ? { text: "NULL", cls: "null", raw: null } : { text: display(v), cls: "", raw: v };
    }
    const pending = edits[r]?.[name];
    if (pending !== undefined) {
      return pending === null ? { text: "NULL", cls: "null", raw: null } : { text: display(pending), cls: "", raw: pending };
    }
    const v = rows[r][c];
    const cls = v === null || isBinary(v) ? "null" : typeof v === "number" ? "num" : "";
    return { text: display(v), cls, raw: v };
  }

  function cellEdited(r: number, c: number) {
    return r < rows.length && edits[r]?.[columns[c].name] !== undefined;
  }

  function rowClass(r: number) {
    if (r >= rows.length) return "inserted";
    if (deleted[r]) return "deleted";
    return "";
  }

  function rowSelected(r: number) {
    if (rowSel) {
      const [a, b] = rowSel.from <= rowSel.to ? [rowSel.from, rowSel.to] : [rowSel.to, rowSel.from];
      return r >= a && r <= b;
    }
    return cur?.row === r;
  }

  function selectCell(r: number, c: number, e?: MouseEvent) {
    menu = null;
    if (e?.shiftKey && anchorRow !== null) {
      rowSel = { from: anchorRow, to: r };
      cur = { row: r, col: c };
      return;
    }
    cur = { row: r, col: c };
    anchorRow = r;
    rowSel = null;
    scroller?.focus();
  }

  function selectRow(r: number, e: MouseEvent) {
    menu = null;
    if (e.shiftKey && anchorRow !== null) rowSel = { from: anchorRow, to: r };
    else {
      anchorRow = r;
      rowSel = { from: r, to: r };
    }
    cur = { row: r, col: cur?.col ?? 0 };
    scroller?.focus();
  }

  function canEdit(r: number, c: number) {
    if (!editable || (r < rows.length && deleted[r])) return false;
    const v = r < rows.length ? rows[r][c] : null;
    return !isBinary(v);
  }

  async function startEdit(r: number, c: number, initial?: string) {
    if (!canEdit(r, c)) return;
    const v = valueAt(r, c).raw;
    const text = initial ?? (v === undefined || v === null ? "" : typeof v === "string" ? v : editText(v as Cell));
    editing = { row: r, col: c, text };
    await tick();
    const input = scroller?.querySelector<HTMLInputElement>("input.editor");
    input?.focus();
    if (initial === undefined) input?.select();
  }

  function commitEdit(move: "down" | "right" | null) {
    if (!editing) return;
    const { row, col, text } = editing;
    editing = null;
    const before = valueAt(row, col).raw;
    const beforeText = before === undefined || before === null ? null : typeof before === "string" ? before : editText(before as Cell);
    // Typing into an empty NULL cell and leaving it empty keeps NULL.
    if (!(before === null && text === "") && text !== beforeText) onedit?.(keyOf(row), columns[col].name, text);
    if (move === "down") moveTo(row + 1, col);
    else if (move === "right") moveTo(row, col + 1);
    scroller?.focus();
  }

  function moveTo(r: number, c: number) {
    r = Math.max(0, Math.min(total - 1, r));
    c = Math.max(0, Math.min(columns.length - 1, c));
    cur = { row: r, col: c };
    anchorRow = r;
    rowSel = null;
    scrollIntoView(r, c);
  }

  function scrollIntoView(r: number, c: number) {
    if (!scroller) return;
    const top = r * ROW_H;
    const bottom = top + ROW_H + HEAD_H;
    if (top < scroller.scrollTop) scroller.scrollTop = top;
    else if (bottom > scroller.scrollTop + scroller.clientHeight) scroller.scrollTop = bottom - scroller.clientHeight;
    const left = GUTTER + widths.slice(0, c).reduce((a, b) => a + b, 0);
    const right = left + widths[c];
    if (left - GUTTER < scroller.scrollLeft) scroller.scrollLeft = left - GUTTER;
    else if (right > scroller.scrollLeft + scroller.clientWidth) scroller.scrollLeft = right - scroller.clientWidth;
  }

  function selectedRowIndexes(): number[] {
    if (rowSel) {
      const [a, b] = rowSel.from <= rowSel.to ? [rowSel.from, rowSel.to] : [rowSel.to, rowSel.from];
      return Array.from({ length: b - a + 1 }, (_, i) => a + i).filter((r) => r < total);
    }
    return cur ? [cur.row] : [];
  }

  function textFor(r: number, c: number) {
    const v = valueAt(r, c).raw;
    if (v === undefined || v === null) return "";
    return typeof v === "string" ? v : isBinary(v as Cell) ? "0x" + (v as { hex: string }).hex : String(v);
  }

  function copy() {
    let text = "";
    if (rowSel) {
      text = selectedRowIndexes()
        .map((r) => columns.map((_, c) => textFor(r, c)).join("\t"))
        .join("\n");
    } else if (cur) text = textFor(cur.row, cur.col);
    navigator.clipboard.writeText(text);
    menu = null;
  }

  function setValue(v: Pending) {
    menu = null;
    if (!cur || !canEdit(cur.row, cur.col)) return;
    onedit?.(keyOf(cur.row), columns[cur.col].name, v);
  }

  function deleteSelected() {
    menu = null;
    if (!editable) return;
    const keys = selectedRowIndexes().map(keyOf);
    if (keys.length) ondelete?.(keys);
  }

  function onkeydown(e: KeyboardEvent) {
    const mod = e.metaKey || e.ctrlKey;
    if (editing) {
      // Keys typed before the editor input has focus still belong to it.
      if (!(e.target as Element).matches?.("input.editor")) {
        if (!mod && e.key.length === 1) {
          e.preventDefault();
          editing.text += e.key;
        } else if (e.key === "Enter") {
          e.preventDefault();
          commitEdit("down");
        }
      }
      return;
    }
    if (mod && e.key === "c") {
      e.preventDefault();
      copy();
      return;
    }
    if (mod && e.key === "a") {
      e.preventDefault();
      anchorRow = 0;
      rowSel = { from: 0, to: total - 1 };
      return;
    }
    if (!cur) {
      if (["ArrowDown", "ArrowUp", "ArrowLeft", "ArrowRight"].includes(e.key) && total > 0) {
        e.preventDefault();
        moveTo(0, 0);
      }
      return;
    }
    const { row, col } = cur;
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        if (e.shiftKey) {
          const to = Math.min(total - 1, (rowSel?.to ?? row) + 1);
          rowSel = { from: anchorRow ?? row, to };
          cur = { row: to, col };
          scrollIntoView(to, col);
        } else moveTo(row + 1, col);
        break;
      case "ArrowUp":
        e.preventDefault();
        if (e.shiftKey) {
          const to = Math.max(0, (rowSel?.to ?? row) - 1);
          rowSel = { from: anchorRow ?? row, to };
          cur = { row: to, col };
          scrollIntoView(to, col);
        } else moveTo(row - 1, col);
        break;
      case "ArrowLeft":
        e.preventDefault();
        moveTo(row, col - 1);
        break;
      case "ArrowRight":
      case "Tab":
        e.preventDefault();
        moveTo(row, col + (e.key === "Tab" && e.shiftKey ? -1 : 1));
        break;
      case "Enter":
        e.preventDefault();
        startEdit(row, col);
        break;
      case "Escape":
        rowSel = null;
        menu = null;
        break;
      case "Delete":
      case "Backspace":
        if (editable && (rowSel || mod)) {
          e.preventDefault();
          deleteSelected();
        }
        break;
      default:
        if (!mod && !e.altKey && e.key.length === 1 && canEdit(row, col)) {
          e.preventDefault();
          startEdit(row, col, e.key);
        }
    }
  }

  function editorKey(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      commitEdit("down");
    } else if (e.key === "Tab") {
      e.preventDefault();
      commitEdit("right");
    } else if (e.key === "Escape") {
      e.preventDefault();
      editing = null;
      scroller?.focus();
    }
    e.stopPropagation();
  }

  function oncontextmenu(e: MouseEvent, r: number, c: number) {
    e.preventDefault();
    if (!rowSelected(r)) selectCell(r, c);
    else cur = { row: r, col: c };
    const box = scroller!.getBoundingClientRect();
    menu = { x: Math.min(e.clientX, box.right - 180), y: Math.min(e.clientY, box.bottom - 170) };
  }

  function startResize(e: MouseEvent, c: number) {
    e.preventDefault();
    e.stopPropagation();
    const startX = e.clientX;
    const startW = widths[c];
    const move = (ev: MouseEvent) => (widths[c] = Math.max(40, startW + ev.clientX - startX));
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  }

  export function focus() {
    scroller?.focus();
  }

  export function selectLast() {
    if (total > 0) moveTo(total - 1, 0);
    scroller?.focus();
  }
</script>

<svelte:window onmousedown={(e) => menu && !(e.target as Element).closest(".menu") && (menu = null)} />

<div
  class="grid"
  bind:this={scroller}
  bind:clientHeight={viewH}
  onscroll={(e) => (scrollTop = e.currentTarget.scrollTop)}
  {onkeydown}
  tabindex="0"
  role="grid"
  aria-rowcount={total}
  aria-colcount={columns.length}
>
  <div class="inner" style:height="{HEAD_H + total * ROW_H}px" style:width="{fullWidth}px">
    <div class="head" style:height="{HEAD_H}px" role="row">
      <div class="gutter hcell" style:width="{GUTTER}px"></div>
      {#each columns as c, i}
        <div class="hcell" style:width="{widths[i]}px" role="columnheader" title="{c.name} ({c.type_name})">
          <button class="hbtn" onclick={() => onsort?.(c.name)} disabled={!onsort}>
            <span class="hname">{c.name}</span>
            <span class="htype">{c.type_name.toLowerCase()}</span>
            {#if sort?.column === c.name}<span class="arrow">{sort.desc ? "▼" : "▲"}</span>{/if}
          </button>
          <!-- svelte-ignore a11y_no_noninteractive_element_interactions (pointer-only resize handle) -->
          <span
            class="resize"
            role="separator"
            aria-orientation="vertical"
            aria-label="Resize {c.name}"
            onmousedown={(e) => startResize(e, i)}
          ></span>
        </div>
      {/each}
    </div>

    {#each visible as r (r)}
      <div
        class="row {rowClass(r)}"
        class:selected={rowSelected(r)}
        style:top="{HEAD_H + r * ROW_H}px"
        style:height="{ROW_H}px"
        role="row"
      >
        <!-- Keyboard navigation lives on the grid container (one tab stop), not on each cell. -->
        <!-- svelte-ignore a11y_interactive_supports_focus -->
        <div class="gutter cell" style:width="{GUTTER}px" onmousedown={(e) => selectRow(r, e)} role="rowheader">
          {r < rows.length ? r + 1 + rowOffset : "+"}
        </div>
        {#each columns as _, c}
          {@const v = valueAt(r, c)}
          <!-- svelte-ignore a11y_interactive_supports_focus -->
          <div
            class="cell {v.cls}"
            class:edited={cellEdited(r, c)}
            class:current={cur?.row === r && cur?.col === c}
            style:width="{widths[c]}px"
            role="gridcell"
            onmousedown={(e) => e.button === 0 && selectCell(r, c, e)}
            ondblclick={() => startEdit(r, c)}
            oncontextmenu={(e) => oncontextmenu(e, r, c)}
          >
            {#if editing?.row === r && editing?.col === c}
              <input
                class="editor"
                bind:value={editing.text}
                onkeydown={editorKey}
                onblur={() => commitEdit(null)}
                aria-label="Edit {columns[c].name}"
              />
            {:else}
              {v.text}
            {/if}
          </div>
        {/each}
      </div>
    {/each}
  </div>

  {#if total === 0}
    <p class="no-rows muted">No rows.</p>
  {/if}
</div>

{#if menu}
  <div class="menu" style:left="{menu.x}px" style:top="{menu.y}px" role="menu">
    <button role="menuitem" onclick={copy}>Copy{rowSel ? " rows" : ""} <span class="kbd">⌘C</span></button>
    {#if cur}
      <button role="menuitem" onclick={() => { navigator.clipboard.writeText(columns[cur!.col].name); menu = null; }}
        >Copy column name</button
      >
    {/if}
    {#if editable}
      <hr />
      <button role="menuitem" onclick={() => cur && startEdit(cur.row, cur.col)} disabled={!cur || !canEdit(cur.row, cur.col)}
        >Edit cell <span class="kbd">↵</span></button
      >
      <button role="menuitem" onclick={() => setValue(null)} disabled={!cur || !canEdit(cur.row, cur.col)}>Set NULL</button>
      <button role="menuitem" onclick={() => setValue("")} disabled={!cur || !canEdit(cur.row, cur.col)}
        >Set empty string</button
      >
      <hr />
      <button role="menuitem" class="danger" onclick={deleteSelected}>Delete row{selectedRows.length > 1 ? "s" : ""}</button>
    {/if}
  </div>
{/if}

<style>
  .grid {
    position: relative;
    flex: 1;
    overflow: auto;
    font-family: var(--mono);
    font-size: 12px;
    outline: none;
    background: var(--panel);
  }
  .inner {
    position: relative;
    min-width: 100%;
  }
  .head {
    position: sticky;
    top: 0;
    z-index: 3;
    display: flex;
    background: var(--bg);
    border-bottom: 1px solid var(--border);
  }
  .hcell {
    position: relative;
    flex: none;
    border-right: 1px solid var(--grid-line);
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif;
  }
  .head .gutter {
    position: sticky;
    left: 0;
    z-index: 4;
    background: var(--bg);
  }
  .hbtn {
    display: flex;
    align-items: baseline;
    gap: 6px;
    width: 100%;
    height: 100%;
    padding: 0 8px;
    border: 0;
    background: none;
    text-align: left;
    overflow: hidden;
  }
  .hbtn:disabled {
    cursor: default;
  }
  .hname {
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    line-height: 26px;
  }
  .htype {
    font-size: 10.5px;
    color: var(--muted);
    white-space: nowrap;
  }
  .arrow {
    font-size: 9px;
    color: var(--accent);
    margin-left: auto;
  }
  .resize {
    position: absolute;
    right: -3px;
    top: 0;
    width: 6px;
    height: 100%;
    cursor: col-resize;
    z-index: 1;
  }
  .row {
    position: absolute;
    left: 0;
    display: flex;
    min-width: 100%;
  }
  .cell {
    flex: none;
    padding: 0 8px;
    line-height: 24px;
    border-right: 1px solid var(--grid-line);
    border-bottom: 1px solid var(--grid-line);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    position: relative;
    cursor: default;
  }
  .cell.num {
    text-align: right;
  }
  .cell.null {
    color: var(--null);
  }
  .gutter.cell {
    position: sticky;
    left: 0;
    z-index: 2;
    text-align: right;
    color: var(--muted);
    background: var(--bg);
    font-size: 11px;
  }
  .row.selected .cell {
    background: var(--selection);
  }
  .row.selected .gutter.cell {
    background: var(--selection);
  }
  .cell.edited,
  .row.selected .cell.edited {
    background: var(--edited);
  }
  .row.inserted .cell:not(.gutter) {
    background: var(--inserted);
  }
  .row.deleted .cell:not(.gutter) {
    background: var(--deleted);
    text-decoration: line-through;
    color: var(--muted);
  }
  .cell.current {
    box-shadow: inset 0 0 0 2px var(--accent);
  }
  .editor {
    position: absolute;
    inset: 0;
    width: 100%;
    border: 2px solid var(--accent);
    padding: 0 6px;
    font: inherit;
    background: var(--panel);
    outline: none;
  }
  .no-rows {
    position: absolute;
    top: 40px;
    left: 0;
    right: 0;
    text-align: center;
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif;
  }
  .menu {
    position: fixed;
    z-index: 50;
    min-width: 180px;
    padding: 4px;
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: 0 8px 28px rgb(0 0 0 / 0.2);
  }
  .menu button {
    display: flex;
    justify-content: space-between;
    width: 100%;
    padding: 4px 8px;
    border: 0;
    border-radius: 4px;
    background: none;
    text-align: left;
  }
  .menu button:hover:not(:disabled) {
    background: var(--accent);
    color: var(--accent-text);
  }
  .menu button:disabled {
    color: var(--muted);
    cursor: default;
  }
  .menu .danger {
    color: var(--danger);
  }
  .menu hr {
    border: 0;
    border-top: 1px solid var(--border);
    margin: 4px 0;
  }
</style>
