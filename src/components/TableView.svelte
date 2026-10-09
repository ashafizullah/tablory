<script lang="ts">
  import { onMount, tick } from "svelte";
  import { app, type Tab } from "../lib/state/app.svelte";
  import { api, errorText } from "../lib/api";
  import { toText } from "../lib/cells";
  import type { ColValue, Filter, ResultSet, RowChange, RowsRequest, TableStructure } from "../lib/types";
  import DataGrid, { type Pending, type RowKey } from "./DataGrid.svelte";
  import StructureView from "./StructureView.svelte";
  import SqlPreview from "./SqlPreview.svelte";
  import ExportMenu from "./ExportMenu.svelte";
  import { quoteIdent } from "../lib/export";

  let { tab, active }: { tab: Extract<Tab, { kind: "table" }>; active: boolean } = $props();

  const PAGE = 300;
  const OPS = [
    ["=", "="],
    ["!=", "≠"],
    ["<", "<"],
    [">", ">"],
    ["<=", "≤"],
    [">=", "≥"],
    ["contains", "contains"],
    ["not_contains", "not contains"],
    ["starts_with", "starts with"],
    ["ends_with", "ends with"],
    ["like", "LIKE"],
    ["is_null", "IS NULL"],
    ["is_not_null", "IS NOT NULL"],
  ] as const;

  let view = $state<"data" | "structure">("data");
  let data = $state<ResultSet | null>(null);
  let structure = $state<TableStructure | null>(null);
  let count = $state<number | null>(null);
  let offset = $state(0);
  let sort = $state<{ column: string; desc: boolean } | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let notice = $state<string | null>(null);

  let showFilters = $state(false);
  let rawMode = $state(false);
  let draftFilters = $state<Filter[]>([]);
  let draftRaw = $state("");
  let filters = $state<Filter[]>([]);
  let rawWhere = $state<string | null>(null);

  let edits = $state<Record<number, Record<string, Pending>>>({});
  let deleted = $state<Record<number, boolean>>({});
  let inserted = $state<Record<string, Pending>[]>([]);
  let selectedRows = $state<RowKey[]>([]);
  let preview = $state<string | null>(null);
  let committing = $state(false);
  let grid: DataGrid | undefined = $state();

  const session = $derived(app.session!);
  const pk = $derived(structure?.columns.filter((c) => c.primary_key).map((c) => c.name) ?? []);
  const isView = $derived(app.tables.find((t) => t.name === tab.table.name)?.kind === "view");
  const readOnly = $derived(session.safety === "readonly");
  const editable = $derived(pk.length > 0 && !isView && !readOnly);
  const changeCount = $derived(
    Object.keys(edits).filter((r) => !deleted[+r]).length +
      Object.values(deleted).filter(Boolean).length +
      inserted.length,
  );
  const columnNames = $derived(data?.columns.map((c) => c.name) ?? structure?.columns.map((c) => c.name) ?? []);

  $effect(() => {
    tab.dirty = changeCount > 0;
  });

  function request(): RowsRequest {
    return {
      table: $state.snapshot(tab.table),
      filters: $state.snapshot(filters),
      raw_where: rawWhere,
      sort: $state.snapshot(sort),
      limit: PAGE,
      offset,
    };
  }

  function clearPending() {
    edits = {};
    deleted = {};
    inserted = [];
  }

  async function load({ recount = true } = {}) {
    loading = true;
    error = null;
    try {
      if (!structure) structure = await api.tableStructure(session.id, tab.table);
      const req = request();
      data = await api.fetchRows(session.id, req);
      clearPending();
      if (recount) {
        count = null;
        api
          .countRows(session.id, req)
          .then((n) => (count = n))
          .catch(() => (count = null));
      }
    } catch (e) {
      error = errorText(e);
    } finally {
      loading = false;
    }
  }

  /** Navigation that reloads the page; asks before dropping pending edits. */
  async function guarded(change: () => void, opts = { recount: true }) {
    if (changeCount > 0 && !(await app.ask("Discard unsaved changes?", { ok: "Discard", danger: true }))) return;
    change();
    await load(opts);
  }

  function toggleSort(column: string) {
    guarded(
      () => {
        if (sort?.column !== column) sort = { column, desc: false };
        else if (!sort.desc) sort = { column, desc: true };
        else sort = null;
        offset = 0;
      },
      { recount: false },
    );
  }

  function page(delta: number) {
    guarded(() => (offset = Math.max(0, offset + delta * PAGE)), { recount: false });
  }

  function applyFilters() {
    guarded(() => {
      if (rawMode) {
        filters = [];
        rawWhere = draftRaw.trim() || null;
      } else {
        filters = draftFilters.filter((f) => f.column && (f.op.includes("null") || f.value !== ""));
        rawWhere = null;
      }
      offset = 0;
    });
  }

  function clearFilters() {
    draftFilters = [];
    draftRaw = "";
    guarded(() => {
      filters = [];
      rawWhere = null;
      offset = 0;
    });
  }

  function addFilter() {
    draftFilters.push({ column: columnNames[0] ?? "", op: "=", value: "" });
    showFilters = true;
  }

  function toggleFilters() {
    showFilters = !showFilters;
    if (showFilters && draftFilters.length === 0 && !rawMode) addFilter();
  }

  function onedit(row: RowKey, column: string, value: Pending) {
    if (row.kind === "new") {
      inserted[row.index][column] = value;
      return;
    }
    const original = data!.rows[row.index][data!.columns.findIndex((c) => c.name === column)];
    const rowEdits = { ...(edits[row.index] ?? {}) };
    if (value === toText(original) && (value !== null || original === null)) delete rowEdits[column];
    else rowEdits[column] = value;
    if (Object.keys(rowEdits).length) edits[row.index] = rowEdits;
    else delete edits[row.index];
  }

  function ondelete(rows: RowKey[]) {
    const newOnes = rows.filter((r) => r.kind === "new").map((r) => r.index);
    inserted = inserted.filter((_, i) => !newOnes.includes(i));
    for (const r of rows) if (r.kind === "row") deleted[r.index] = !deleted[r.index];
  }

  function addRow() {
    inserted.push({});
    tick().then(() => grid?.selectLast());
  }

  function keyFor(r: number): ColValue[] {
    const cols = data!.columns.map((c) => c.name);
    return pk.map((name) => ({ column: name, value: toText(data!.rows[r][cols.indexOf(name)]) }));
  }

  function changes(): RowChange[] {
    const out: RowChange[] = [];
    for (const [r, isDeleted] of Object.entries(deleted)) {
      if (isDeleted) out.push({ type: "delete", key: keyFor(+r) });
    }
    for (const [r, values] of Object.entries(edits)) {
      if (deleted[+r]) continue;
      out.push({
        type: "update",
        key: keyFor(+r),
        values: Object.entries(values).map(([column, value]) => ({ column, value })),
      });
    }
    for (const row of inserted) {
      out.push({ type: "insert", values: Object.entries(row).map(([column, value]) => ({ column, value })) });
    }
    return out;
  }

  async function showPreview() {
    try {
      preview = await api.previewChanges(session.id, tab.table, changes());
    } catch (e) {
      error = errorText(e);
    }
  }

  async function commit() {
    if (changeCount === 0 || committing) return;
    if (!(await app.guardEdits(changeCount))) return;
    committing = true;
    error = null;
    try {
      const n = await api.applyChanges(session.id, tab.table, changes());
      preview = null;
      notice = `Saved: ${n} row${n === 1 ? "" : "s"} changed.`;
      setTimeout(() => (notice = null), 3000);
      await load();
    } catch (e) {
      error = errorText(e);
      preview = null;
    } finally {
      committing = false;
    }
  }

  async function discard() {
    clearPending();
  }

  function onkeydown(e: KeyboardEvent) {
    if (!active || app.confirm || !(e.metaKey || e.ctrlKey)) return;
    if (e.key === "s") {
      e.preventDefault();
      commit();
    } else if (e.key === "r") {
      e.preventDefault();
      guarded(() => {});
    }
  }

  onMount(() => {
    load();
  });

  const rangeText = $derived.by(() => {
    if (!data) return "";
    const n = data.rows.length;
    if (n === 0) return offset > 0 ? `No rows after ${offset.toLocaleString()}` : "0 rows";
    const end = offset + n;
    return `${(offset + 1).toLocaleString()}–${end.toLocaleString()}${count !== null ? ` of ${count.toLocaleString()}` : ""}`;
  });
  const hasNext = $derived(data ? (count !== null ? offset + data.rows.length < count : data.rows.length === PAGE) : false);
  const filtered = $derived(filters.length > 0 || rawWhere !== null);
</script>

<svelte:window {onkeydown} />

<div class="tv">
  {#if view === "data"}
    {#if showFilters}
      <div class="filters">
        {#if rawMode}
          <div class="frow">
            <span class="kw">WHERE</span>
            <input
              class="field grow mono"
              bind:value={draftRaw}
              placeholder="status = 'active' AND created_at > now() - interval '7 days'"
              aria-label="WHERE clause"
              onkeydown={(e) => e.key === "Enter" && applyFilters()}
            />
          </div>
        {:else}
          {#each draftFilters as f, i}
            <div class="frow">
              <select class="field col" bind:value={f.column} aria-label="Column">
                {#each columnNames as c}<option value={c}>{c}</option>{/each}
              </select>
              <select class="field op" bind:value={f.op} aria-label="Operator">
                {#each OPS as [v, label]}<option value={v}>{label}</option>{/each}
              </select>
              {#if !f.op.includes("null")}
                <input
                  class="field grow mono"
                  bind:value={f.value}
                  placeholder="Value"
                  aria-label="Value"
                  onkeydown={(e) => e.key === "Enter" && applyFilters()}
                />
              {:else}
                <span class="grow"></span>
              {/if}
              <button class="btn ghost" aria-label="Remove filter" onclick={() => draftFilters.splice(i, 1)}>−</button>
            </div>
          {/each}
        {/if}
        <div class="frow">
          {#if !rawMode}<button class="btn" onclick={addFilter}>Add condition</button>{/if}
          <button class="btn ghost" onclick={() => (rawMode = !rawMode)}>{rawMode ? "Use conditions" : "Write SQL"}</button>
          <span class="grow"></span>
          {#if filtered}<button class="btn" onclick={clearFilters}>Clear</button>{/if}
          <button class="btn primary" onclick={applyFilters}>Apply</button>
        </div>
      </div>
    {/if}

    {#if !editable && structure && data}
      <div class="banner muted">
        {readOnly
          ? `Read-only: ${session.name} doesn't allow changes.`
          : isView
            ? "Views are read-only."
            : "Read-only: this table has no primary key, so rows can't be matched safely for edits."}
      </div>
    {/if}

    {#if error}
      <div class="banner error-text">{error}</div>
    {/if}

    {#if data}
      <DataGrid
        bind:this={grid}
        bind:selectedRows
        columns={data.columns}
        rows={data.rows}
        {inserted}
        {edits}
        {deleted}
        {editable}
        rowOffset={offset}
        {sort}
        onsort={toggleSort}
        {onedit}
        {ondelete}
      />
    {:else if loading}
      <div class="placeholder muted">Loading {tab.table.name}…</div>
    {:else if !error}
      <div class="placeholder muted">No data loaded.</div>
    {/if}
  {:else if structure}
    <StructureView {structure} />
  {:else}
    <div class="placeholder muted">{error ?? "Loading structure…"}</div>
  {/if}

  <footer class="bar">
    <div class="seg" role="tablist">
      <button role="tab" aria-selected={view === "data"} class:on={view === "data"} onclick={() => (view = "data")}>Data</button>
      <button role="tab" aria-selected={view === "structure"} class:on={view === "structure"} onclick={() => (view = "structure")}
        >Structure</button
      >
    </div>
    {#if view === "data"}
      <button class="btn" class:on={showFilters || filtered} onclick={toggleFilters}>
        Filter{filtered ? ` (${rawWhere !== null ? "SQL" : filters.length})` : ""}
      </button>
      {#if editable}
        <button class="btn" onclick={addRow} title="Insert a row">+ Row</button>
        <button class="btn" onclick={() => ondelete(selectedRows)} disabled={selectedRows.length === 0} title="Delete selected rows (⌘⌫)"
          >Delete</button
        >
      {/if}
    {/if}

    <span class="grow"></span>

    {#if changeCount > 0}
      <span class="pending">{changeCount} unsaved change{changeCount === 1 ? "" : "s"}</span>
      <button class="btn" onclick={discard}>Discard</button>
      <button class="btn" onclick={showPreview}>Preview SQL</button>
      <button class="btn primary" onclick={commit} disabled={committing}>{committing ? "Saving…" : "Commit"} <span class="kbd on-accent">⌘S</span></button>
    {:else if notice}
      <span class="saved" role="status">{notice}</span>
    {/if}

    {#if view === "data" && data && changeCount === 0}
      <ExportMenu
        kind={session.kind}
        name={tab.table.name}
        table={`${quoteIdent(session.kind, tab.table.schema)}.${quoteIdent(session.kind, tab.table.name)}`}
        columns={data.columns.map((c) => c.name)}
        rows={data.rows}
        truncated={hasNext || offset > 0}
      />
    {/if}
    {#if view === "data"}
      <span class="range muted">{loading ? "Loading…" : rangeText}</span>
      <button class="btn ghost" aria-label="Previous page" onclick={() => page(-1)} disabled={offset === 0 || loading}>‹</button>
      <button class="btn ghost" aria-label="Next page" onclick={() => page(1)} disabled={!hasNext || loading}>›</button>
      <button class="btn ghost" aria-label="Reload (⌘R)" title="Reload (⌘R)" onclick={() => guarded(() => {})}>↻</button>
    {/if}
  </footer>
</div>

{#if preview !== null}
  <SqlPreview sql={preview} {committing} oncommit={commit} onclose={() => (preview = null)} />
{/if}

<style>
  .tv {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .filters {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px;
    border-bottom: 1px solid var(--border);
    background: var(--bg);
  }
  .frow {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .col {
    width: 180px;
  }
  .op {
    width: 120px;
  }
  .grow {
    flex: 1;
  }
  .mono {
    font-family: var(--mono);
    font-size: 12px;
  }
  .kw {
    font-family: var(--mono);
    font-size: 12px;
    color: var(--syn-keyword);
  }
  .banner {
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
    font-size: 12px;
  }
  .placeholder {
    flex: 1;
    display: grid;
    place-items: center;
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
  .seg {
    display: flex;
    border: 1px solid var(--border);
    border-radius: 6px;
    overflow: hidden;
  }
  .seg button {
    border: 0;
    background: var(--panel);
    padding: 0 10px;
    height: 24px;
  }
  .seg button + button {
    border-left: 1px solid var(--border);
  }
  .seg button.on {
    background: var(--selection);
  }
  .btn.on {
    border-color: var(--accent);
  }
  .pending {
    color: #b45309;
    font-size: 12px;
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
