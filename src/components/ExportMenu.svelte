<script lang="ts">
  import { api, errorText } from "../lib/api";
  import { insertStatements } from "../lib/export";
  import type { Cell, DbKind } from "../lib/types";

  let {
    kind,
    name,
    table,
    columns,
    rows,
    truncated = false,
  }: {
    kind: DbKind;
    /** Default file name, without extension. */
    name: string;
    /** Quoted target table for INSERT statements. */
    table: string;
    columns: string[];
    rows: Cell[][];
    /** Only part of the result was fetched. */
    truncated?: boolean;
  } = $props();

  let open = $state(false);
  let busy = $state(false);
  let notice = $state<{ ok: boolean; text: string } | null>(null);
  let timer: ReturnType<typeof setTimeout> | undefined;

  function say(ok: boolean, text: string) {
    notice = { ok, text };
    clearTimeout(timer);
    timer = setTimeout(() => (notice = null), ok ? 4000 : 8000);
  }

  const count = $derived(`${rows.length.toLocaleString()} row${rows.length === 1 ? "" : "s"}`);

  async function save(format: "csv" | "json" | "xlsx") {
    open = false;
    busy = true;
    try {
      const path = await api.exportResult(format, name, columns, rows);
      if (path) say(true, `Exported ${count} to ${path}`);
    } catch (e) {
      say(false, `Export failed: ${errorText(e)}`);
    } finally {
      busy = false;
    }
  }

  async function copyInserts() {
    open = false;
    try {
      await navigator.clipboard.writeText(insertStatements(kind, table, columns, rows));
      say(true, `Copied ${count} as INSERT statements`);
    } catch (e) {
      say(false, `Copy failed: ${errorText(e)}`);
    }
  }
</script>

<svelte:window onmousedown={(e) => open && !(e.target as Element).closest(".export") && (open = false)} />

<span class="export">
  {#if notice}<span class="notice" class:error-text={!notice.ok} title={notice.text}>{notice.text}</span>{/if}
  <button
    class="btn small"
    onclick={() => (open = !open)}
    disabled={busy || columns.length === 0}
    aria-haspopup="menu"
    aria-expanded={open}
    title={truncated ? "Exports the rows fetched so far" : "Export these rows"}>{busy ? "Exporting…" : "Export ▾"}</button
  >
  {#if open}
    <div class="menu" role="menu">
      <button role="menuitem" onclick={() => save("csv")}>CSV…</button>
      <button role="menuitem" onclick={() => save("xlsx")}>Excel (.xlsx)…</button>
      <button role="menuitem" onclick={() => save("json")}>JSON…</button>
      <hr />
      <button role="menuitem" onclick={copyInserts}>Copy as INSERT</button>
      {#if truncated}
        <p class="muted hint">Only the {count} shown here are exported, not the whole result.</p>
      {/if}
    </div>
  {/if}
</span>

<style>
  .export {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .small {
    height: 20px;
    padding: 0 8px;
    font-size: 12px;
  }
  .notice {
    max-width: 360px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .menu {
    position: absolute;
    right: 0;
    bottom: calc(100% + 4px);
    z-index: 50;
    min-width: 190px;
    padding: 4px;
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: 0 8px 28px rgb(0 0 0 / 0.2);
  }
  .menu button {
    display: block;
    width: 100%;
    padding: 4px 8px;
    border: 0;
    border-radius: 4px;
    background: none;
    color: var(--text);
    text-align: left;
    font-size: 13px;
  }
  .menu button:hover {
    background: var(--accent);
    color: var(--accent-text);
  }
  .menu hr {
    border: 0;
    border-top: 1px solid var(--border);
    margin: 4px 0;
  }
  .hint {
    margin: 4px 8px 2px;
    font-size: 11px;
    max-width: 200px;
  }
</style>
