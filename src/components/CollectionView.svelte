<script lang="ts">
  import { onMount } from "svelte";
  import { app, type Tab } from "../lib/state/app.svelte";
  import { api, errorText } from "../lib/api";
  import type { Cell, ColumnMeta } from "../lib/types";
  import DataGrid, { type RowKey } from "./DataGrid.svelte";
  import JsonEditor from "./JsonEditor.svelte";

  let { tab, active }: { tab: Extract<Tab, { kind: "collection" }>; active: boolean } = $props();

  const PAGE = 100;
  const MAX_COLUMNS = 60;

  let filter = $state("");
  let sort = $state("");
  let applied = $state({ filter: "", sort: "" });
  let skip = $state(0);
  let docs = $state<Record<string, unknown>[]>([]);
  let texts = $state<string[]>([]);
  let count = $state<number | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let notice = $state<string | null>(null);
  let selectedRows = $state<RowKey[]>([]);

  // Editor: an existing document (index) or a new one.
  let editing = $state<{ index: number | null; original: string; text: string } | null>(null);
  let saving = $state(false);
  // Bumped to reload the editor with the original text.
  let editorKey = $state(0);

  function revert() {
    if (!editing) return;
    editing.text = editing.original;
    editorKey += 1;
  }

  const session = $derived(app.session!);
  const isView = $derived(app.tables.find((t) => t.name === tab.name)?.kind === "view");
  const dirty = $derived(editing !== null && editing.text !== editing.original);

  $effect(() => {
    tab.dirty = dirty;
  });

  /** _id first, then fields in order of first appearance. */
  const columns = $derived.by((): ColumnMeta[] => {
    const seen = new Set<string>(["_id"]);
    const names = ["_id"];
    for (const d of docs) {
      for (const k of Object.keys(d)) {
        if (!seen.has(k) && names.length < MAX_COLUMNS) {
          seen.add(k);
          names.push(k);
        }
      }
    }
    return names.map((name) => ({ name, type_name: "" }));
  });

  /** Extended JSON shown the way the mongo shell prints it. */
  function cellOf(v: unknown): Cell {
    if (v === undefined) return "";
    if (v === null || typeof v === "string" || typeof v === "number" || typeof v === "boolean") return v;
    const o = v as Record<string, unknown>;
    if (typeof o.$oid === "string") return `ObjectId('${o.$oid}')`;
    if (o.$date !== undefined) return typeof o.$date === "string" ? o.$date : JSON.stringify(o.$date);
    if (typeof o.$numberLong === "string") return o.$numberLong;
    if (typeof o.$numberDecimal === "string") return o.$numberDecimal;
    return JSON.stringify(v);
  }

  const rows = $derived(docs.map((d) => columns.map((c) => cellOf(d[c.name]))));

  async function load({ recount = true } = {}) {
    loading = true;
    error = null;
    try {
      const res = await api.mongoFind(session.id, tab.db, tab.name, applied.filter, applied.sort, skip, PAGE);
      docs = res.docs;
      texts = res.texts;
      editing = null;
      if (recount) {
        count = null;
        api
          .mongoCount(session.id, tab.db, tab.name, applied.filter)
          .then((n) => (count = n))
          .catch(() => (count = null));
      }
    } catch (e) {
      error = errorText(e);
    } finally {
      loading = false;
    }
  }

  async function guarded(change: () => void, recount = true) {
    if (dirty && !(await app.ask("Discard changes to this document?", { ok: "Discard", danger: true }))) return;
    change();
    await load({ recount });
  }

  function apply() {
    guarded(() => {
      applied = { filter: filter.trim(), sort: sort.trim() };
      skip = 0;
    });
  }

  function page(delta: number) {
    guarded(() => (skip = Math.max(0, skip + delta * PAGE)), false);
  }

  // Selecting a row opens it, unless the open document has unsaved edits.
  $effect(() => {
    const sel = selectedRows;
    if (sel.length !== 1 || sel[0].kind !== "row") return;
    const index = sel[0].index;
    if (editing?.index === index || dirty) return;
    editing = { index, original: texts[index], text: texts[index] };
  });

  function newDocument() {
    const text = "{\n  \n}";
    editing = { index: null, original: text, text };
  }

  async function save() {
    if (!editing || saving) return;
    saving = true;
    error = null;
    try {
      if (editing.index === null) {
        await api.mongoInsert(session.id, tab.db, tab.name, editing.text);
        notice = "Document inserted.";
        await load();
      } else {
        const id = JSON.stringify(docs[editing.index]._id);
        await api.mongoReplace(session.id, tab.db, tab.name, id, editing.text);
        notice = "Document saved.";
        await load({ recount: false });
      }
      setTimeout(() => (notice = null), 3000);
    } catch (e) {
      error = errorText(e);
    } finally {
      saving = false;
    }
  }

  async function remove() {
    const indexes = selectedRows.filter((r) => r.kind === "row").map((r) => r.index);
    if (indexes.length === 0) return;
    const n = indexes.length;
    const ok = await app.ask(`Delete ${n} document${n === 1 ? "" : "s"}?`, {
      detail: "This cannot be undone.",
      ok: "Delete",
      danger: true,
    });
    if (!ok) return;
    try {
      const ids = indexes.map((i) => JSON.stringify(docs[i]._id));
      const deleted = await api.mongoDelete(session.id, tab.db, tab.name, ids);
      notice = `Deleted ${deleted} document${deleted === 1 ? "" : "s"}.`;
      setTimeout(() => (notice = null), 3000);
      editing = null;
      await load();
    } catch (e) {
      error = errorText(e);
    }
  }

  function onkeydown(e: KeyboardEvent) {
    if (!active || app.confirm || !(e.metaKey || e.ctrlKey)) return;
    if (e.key === "s" && editing) {
      e.preventDefault();
      save();
    } else if (e.key === "r") {
      e.preventDefault();
      guarded(() => {});
    }
  }

  onMount(() => load());

  const rangeText = $derived.by(() => {
    if (docs.length === 0) return skip > 0 ? `No documents after ${skip.toLocaleString()}` : "0 documents";
    const end = skip + docs.length;
    return `${(skip + 1).toLocaleString()}–${end.toLocaleString()}${count !== null ? ` of ${count.toLocaleString()}` : ""}`;
  });
  const hasNext = $derived(count !== null ? skip + docs.length < count : docs.length === PAGE);
  const filtered = $derived(applied.filter !== "" || applied.sort !== "");
</script>

<svelte:window {onkeydown} />

<div class="cv">
  <div class="query">
    <label class="lbl" for="filter-{tab.id}">Filter</label>
    <input
      id="filter-{tab.id}"
      class="field mono grow"
      bind:value={filter}
      placeholder={'{ status: "active", age: { $gt: 30 } }'}
      onkeydown={(e) => e.key === "Enter" && apply()}
    />
    <label class="lbl" for="sort-{tab.id}">Sort</label>
    <input
      id="sort-{tab.id}"
      class="field mono sort"
      bind:value={sort}
      placeholder={"{ _id: -1 }"}
      onkeydown={(e) => e.key === "Enter" && apply()}
    />
    <button class="btn primary" onclick={apply}>Apply</button>
  </div>

  {#if error}<div class="banner error-text">{error}</div>{/if}
  {#if isView}<div class="banner muted">Views are read-only.</div>{/if}

  <div class="split">
    <div class="grid-pane">
      {#if loading && docs.length === 0}
        <div class="placeholder muted">Loading {tab.name}…</div>
      {:else if docs.length === 0 && !error}
        <div class="placeholder muted">
          {filtered ? "No documents match this filter." : "This collection is empty."}
          {#if !isView}<button class="btn" onclick={newDocument}>Insert a document</button>{/if}
        </div>
      {:else}
        <DataGrid {columns} {rows} bind:selectedRows rowOffset={skip} />
      {/if}
    </div>
    {#if editing}
      <aside class="doc">
        <div class="doc-head">
          <strong>{editing.index === null ? "New document" : "Document"}</strong>
          {#if dirty}<span class="pending">unsaved</span>{/if}
          <span class="grow"></span>
          <button class="btn ghost" aria-label="Close editor" onclick={() => guarded(() => {}, false)}>×</button>
        </div>
        {#key `${editing.index}-${editorKey}`}
          <JsonEditor value={editing.original} onchange={(v) => editing && (editing.text = v)} onsubmit={save} readonly={isView} />
        {/key}
        {#if !isView}
          <div class="doc-actions">
            {#if dirty}
              <button class="btn" onclick={revert}>Revert</button>
            {/if}
            <span class="grow"></span>
            <button class="btn primary" onclick={save} disabled={saving || (!dirty && editing.index !== null)}>
              {saving ? "Saving…" : editing.index === null ? "Insert" : "Save"} <span class="kbd on-accent">⌘S</span>
            </button>
          </div>
        {/if}
      </aside>
    {/if}
  </div>

  <footer class="bar">
    {#if !isView}
      <button class="btn" onclick={newDocument}>+ Document</button>
      <button class="btn" onclick={remove} disabled={selectedRows.filter((r) => r.kind === "row").length === 0}>Delete</button>
    {/if}
    <span class="grow"></span>
    {#if notice}<span class="saved" role="status">{notice}</span>{/if}
    <span class="range muted">{loading ? "Loading…" : rangeText}</span>
    <button class="btn ghost" aria-label="Previous page" onclick={() => page(-1)} disabled={skip === 0 || loading}>‹</button>
    <button class="btn ghost" aria-label="Next page" onclick={() => page(1)} disabled={!hasNext || loading}>›</button>
    <button class="btn ghost" aria-label="Reload (⌘R)" title="Reload (⌘R)" onclick={() => guarded(() => {})}>↻</button>
  </footer>
</div>

<style>
  .cv {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .query {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px;
    border-bottom: 1px solid var(--border);
    background: var(--bg);
  }
  .lbl {
    color: var(--muted);
    font-size: 12px;
  }
  .mono {
    font-family: var(--mono);
    font-size: 12px;
  }
  .grow {
    flex: 1;
  }
  .sort {
    width: 180px;
  }
  .banner {
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
    font-size: 12px;
  }
  .split {
    flex: 1;
    display: flex;
    min-height: 0;
  }
  .grid-pane {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .placeholder {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 8px;
    align-items: center;
    justify-content: center;
  }
  .doc {
    width: min(420px, 45%);
    display: flex;
    flex-direction: column;
    border-left: 1px solid var(--border);
    min-height: 0;
  }
  .doc-head,
  .doc-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 8px;
    background: var(--bg);
  }
  .doc-head {
    border-bottom: 1px solid var(--border);
  }
  .doc-actions {
    border-top: 1px solid var(--border);
  }
  .pending {
    color: #b45309;
    font-size: 12px;
  }
  .bar {
    flex: none;
    display: flex;
    align-items: center;
    gap: 6px;
    height: 36px;
    padding: 0 8px;
    border-top: 1px solid var(--border);
    background: var(--bg);
  }
  .saved {
    color: #16a34a;
    font-size: 12px;
  }
  .range {
    font-size: 12px;
    margin-left: 8px;
    white-space: nowrap;
  }
  .on-accent {
    color: inherit;
    opacity: 0.75;
  }
</style>
