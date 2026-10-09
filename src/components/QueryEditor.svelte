<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { basicSetup } from "codemirror";
  import { Compartment, EditorState, Prec } from "@codemirror/state";
  import { EditorView } from "@codemirror/view";
  import { sql as sqlLang, type SQLNamespace } from "@codemirror/lang-sql";
  import { app, type Tab } from "../lib/state/app.svelte";
  import { api, errorText } from "../lib/api";
  import { kindLabel, uid } from "../lib/cells";
  import { dialectFor, highlight, keymap, theme } from "../lib/codemirror";
  import { splitStatements, statementAt } from "../lib/sqlsplit";
  import type { ExecuteResult } from "../lib/types";
  import { addHistory } from "../lib/history";
  import { quoteIdent, tableFromSql } from "../lib/export";
  import DataGrid from "./DataGrid.svelte";
  import ExportMenu from "./ExportMenu.svelte";
  import HistoryPanel from "./HistoryPanel.svelte";
  import SafetyBadge from "./SafetyBadge.svelte";

  let { tab, active }: { tab: Extract<Tab, { kind: "query" }>; active: boolean } = $props();

  let host: HTMLDivElement | undefined = $state();
  let view: EditorView | undefined;
  const language = new Compartment();

  let running = $state<string | null>(null);
  let result = $state<ExecuteResult | null>(null);
  let error = $state<string | null>(null);
  let ranSql = $state("");
  let resultIndex = $state(0);
  let maxRows = $state(1000);
  let editorHeight = $state(260);
  let showHistory = $state(false);
  let historyVersion = $state(0);

  const session = $derived(app.session!);
  const mysql = $derived(session.kind === "mysql");

  // Where unqualified names resolve, read from the editor's own connection
  // so a SET search_path / USE in this session shows up after a run.
  const SCHEMA_SQL: Partial<Record<string, string>> = {
    postgres: "SELECT current_schema()",
    mssql: "SELECT SCHEMA_NAME()",
  };
  let defaultSchema = $state("");
  async function loadDefaultSchema() {
    const q = SCHEMA_SQL[session.kind];
    if (!q) return;
    try {
      const r = await api.execute(session.id, q, 1, uid());
      const v = r.statements[0]?.result?.rows[0]?.[0];
      defaultSchema = typeof v === "string" ? v : "";
    } catch {
      defaultSchema = "";
    }
  }
  $effect(() => {
    void session.database;
    loadDefaultSchema();
  });

  function languageExt() {
    const schema: SQLNamespace = {};
    for (const t of app.tables) {
      schema[t.name] = (app.columns[t.name] ?? []).map((c) => ({
        label: c.column,
        type: "property",
        detail: c.data_type,
      }));
    }
    return sqlLang({ dialect: dialectFor(session.kind), schema, upperCaseKeywords: true });
  }

  /** Selection if any, else the statement under the cursor. */
  function target(all: boolean): string {
    if (!view) return "";
    const doc = view.state.doc.toString();
    if (all) return doc;
    const sel = view.state.selection.main;
    if (!sel.empty) return doc.slice(sel.from, sel.to);
    const s = statementAt(doc, sel.head, mysql);
    return s ? doc.slice(s.from, s.to) : "";
  }

  async function run(all = false) {
    if (running) return;
    const sql = target(all).trim();
    if (!sql) return;
    if (!(await app.guardSql(sql))) return;
    if (running) return;
    const qid = uid();
    running = qid;
    error = null;
    ranSql = sql;
    const db = session.database;
    try {
      await app.beforeRun(sql);
      result = await api.execute(session.id, sql, maxRows, qid);
      // Show the last statement that returned rows, else the last one.
      let idx = result.statements.length - 1;
      while (idx > 0 && !result.statements[idx].result) idx--;
      resultIndex = result.statements[idx]?.result ? idx : result.statements.length - 1;
      // DDL may have changed the table list.
      if (/^\s*(create|drop|alter|rename)\b/im.test(sql)) app.loadTables();
    } catch (e) {
      result = null;
      error = errorText(e);
    } finally {
      running = null;
    }
    app.afterRun(sql, error);
    remember(sql, db);
    loadDefaultSchema();
  }

  function remember(sql: string, database: string) {
    const last = result?.statements.at(-1);
    const shownResult = result?.statements[resultIndex]?.result;
    addHistory(session.connection_id, {
      sql,
      at: Date.now(),
      database,
      ms: result?.duration_ms ?? 0,
      error: error ?? undefined,
      rows: error ? undefined : shownResult ? shownResult.rows.length : last?.rows_affected,
    });
    historyVersion++;
  }

  /** Puts SQL from the history at the cursor, on its own line. */
  function insertSql(sql: string) {
    if (!view) return;
    const { from, to } = view.state.selection.main;
    const doc = view.state.doc;
    const before = from > 0 && doc.sliceString(from - 1, from) !== "\n" ? "\n" : "";
    const text = before + sql.replace(/;?\s*$/, ";") + "\n";
    view.dispatch({ changes: { from, to, insert: text }, selection: { anchor: from + text.length } });
    view.focus();
  }

  const exportTable = $derived.by(() => {
    const t = tableFromSql(ranSql);
    return t ?? quoteIdent(session.kind, "table_name");
  });

  async function cancel() {
    if (!running) return;
    try {
      await api.cancelQuery(session.id, running);
    } catch (e) {
      error = errorText(e);
    }
  }

  onMount(() => {
    view = new EditorView({
      parent: host!,
      state: EditorState.create({
        doc: tab.sql,
        extensions: [
          Prec.highest(
            keymap.of([
              { key: "Mod-Enter", run: () => (run(false), true) },
              { key: "Shift-Mod-Enter", run: () => (run(true), true) },
            ]),
          ),
          basicSetup,
          language.of(languageExt()),
          theme,
          highlight,
          EditorView.lineWrapping,
          EditorView.updateListener.of((u) => {
            if (u.docChanged) tab.sql = u.state.doc.toString();
          }),
        ],
      }),
    });
    view.focus();
  });

  // Keep autocomplete in sync with the table and column lists.
  $effect(() => {
    void app.tables;
    void app.columns;
    view?.dispatch({ effects: language.reconfigure(languageExt()) });
  });

  $effect(() => {
    if (active) queueMicrotask(() => view?.focus());
  });

  onDestroy(() => view?.destroy());

  function onkeydown(e: KeyboardEvent) {
    if (!active || app.confirm || !(e.metaKey || e.ctrlKey)) return;
    const key = e.key.toLowerCase();
    if (key === "s") {
      e.preventDefault();
      app.saveQuery(tab, e.shiftKey);
    } else if (key === "h" && e.shiftKey) {
      e.preventDefault();
      showHistory = !showHistory;
    }
  }

  function startResize(e: MouseEvent) {
    e.preventDefault();
    const startY = e.clientY;
    const startH = editorHeight;
    const move = (ev: MouseEvent) => (editorHeight = Math.max(80, Math.min(window.innerHeight - 220, startH + ev.clientY - startY)));
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  }

  const current = $derived(result?.statements[resultIndex] ?? null);
  const statementCount = $derived(splitStatements(tab.sql, mysql).length);
</script>

<svelte:window {onkeydown} />

<div class="qe">
  {#if showHistory}
    <HistoryPanel version={historyVersion} oninsert={insertSql} onclose={() => (showHistory = false)} />
  {/if}
  <div class="toolbar">
    <button class="btn primary" onclick={() => run(false)} disabled={!!running} title="Run statement or selection (⌘↵)">
      Run <span class="kbd on-accent">⌘↵</span>
    </button>
    <button class="btn" onclick={() => run(true)} disabled={!!running || statementCount === 0} title="Run all (⇧⌘↵)">
      Run all{statementCount > 1 ? ` (${statementCount})` : ""}
    </button>
    {#if running}
      <button class="btn danger" onclick={cancel} disabled={session.kind === "sqlite"}>Cancel</button>
    {/if}
    <label
      class="check small"
      title={app.manualCommit
        ? "Changes stay in a transaction until you Commit or Roll back"
        : "Each statement is committed as soon as it runs"}
    >
      <input type="checkbox" bind:checked={app.manualCommit} disabled={app.txOpen} /> Manual commit
    </label>
    <span class="grow"></span>
    <button class="btn" class:on={showHistory} onclick={() => (showHistory = !showHistory)} title="Query history (⇧⌘H)"
      >History</button
    >
    <button class="btn" onclick={() => app.saveQuery(tab)} title={tab.path ? `Save to ${tab.path} (⌘S)` : "Save as a .sql file (⌘S)"}>
      Save
    </button>
    <button class="btn" onclick={() => app.saveQuery(tab, true)} title="Save to a new file (⇧⌘S)">Save as…</button>
    <label class="muted small" for="max-{tab.id}">Row limit</label>
    <select id="max-{tab.id}" class="field" bind:value={maxRows}>
      <option value={200}>200</option>
      <option value={1000}>1,000</option>
      <option value={5000}>5,000</option>
      <option value={20000}>20,000</option>
    </select>
  </div>

  <div class="target" style:border-left-color={session.color || "var(--muted)"} title="Queries in this tab run here">
    <span class="muted">Runs on</span>
    <span class="swatch" style:background={session.color || "var(--muted)"}></span>
    <strong>{session.name}</strong>
    <span class="muted">{kindLabel[session.kind]}</span>
    {#if session.safety !== "normal"}<SafetyBadge safety={session.safety} />{/if}
    <span class="sep" aria-hidden="true">›</span>
    <span class="icon" aria-hidden="true">⛁</span>
    <strong>{session.database || "(default database)"}</strong>
    {#if defaultSchema}
      <span class="sep" aria-hidden="true">›</span>
      <span>{defaultSchema}</span>
      <span class="muted">default schema</span>
    {/if}
  </div>

  <div class="editor" bind:this={host} style:height="{editorHeight}px"></div>
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions (pointer-only splitter) -->
  <div class="divider" role="separator" aria-orientation="horizontal" aria-label="Resize editor" onmousedown={startResize}></div>

  <div class="results">
    {#if running}
      <div class="placeholder muted">Running…</div>
    {:else if error}
      <div class="message">
        <p class="error-text">{error}</p>
        <pre class="muted">{ranSql}</pre>
      </div>
    {:else if result}
      {#if result.statements.length > 1}
        <div class="rtabs" role="tablist">
          {#each result.statements as s, i}
            <button role="tab" aria-selected={i === resultIndex} class:on={i === resultIndex} onclick={() => (resultIndex = i)}>
              {s.result ? `Result ${i + 1}` : `Statement ${i + 1}`}
            </button>
          {/each}
        </div>
      {/if}
      {#if current?.result}
        {#key `${resultIndex}-${result.duration_ms}-${ranSql}`}
          <DataGrid columns={current.result.columns} rows={current.result.rows} />
        {/key}
      {:else if current}
        <div class="placeholder muted">
          {current.rows_affected.toLocaleString()} row{current.rows_affected === 1 ? "" : "s"} affected.
        </div>
      {:else}
        <div class="placeholder muted">Done. No statements returned results.</div>
      {/if}
    {:else}
      <div class="placeholder muted">Write a query and press ⌘↵ to run the statement under the cursor.</div>
    {/if}
  </div>

  <footer class="bar muted">
    {#if result && !running}
      {#if current?.result}
        <span>{current.result.rows.length.toLocaleString()} row{current.result.rows.length === 1 ? "" : "s"}</span>
        {#if current.result.truncated}<span class="warn">limited to {maxRows.toLocaleString()} rows</span>{/if}
      {/if}
      <span>{result.duration_ms.toLocaleString()} ms</span>
    {/if}
    <span class="grow"></span>
    {#if current?.result && !running}
      <ExportMenu
        kind={session.kind}
        name={tab.title}
        table={exportTable}
        columns={current.result.columns.map((c) => c.name)}
        rows={current.result.rows}
        truncated={current.result.truncated}
      />
    {/if}
    <span>{session.database}</span>
  </footer>
</div>

<style>
  .qe {
    position: relative;
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 8px;
    border-bottom: 1px solid var(--border);
    background: var(--bg);
  }
  .check {
    display: flex;
    align-items: center;
    gap: 4px;
    margin-left: 6px;
    white-space: nowrap;
  }
  .grow {
    flex: 1;
  }
  .small {
    font-size: 12px;
  }
  .on-accent {
    color: inherit;
    opacity: 0.75;
  }
  .target {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: none;
    padding: 4px 10px;
    border-bottom: 1px solid var(--border);
    border-left: 3px solid;
    background: var(--bg);
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
  }
  .target .swatch {
    width: 8px;
    height: 8px;
    border-radius: 2px;
    flex: none;
  }
  .target .sep,
  .target .icon {
    color: var(--muted);
  }
  .editor {
    flex: none;
    overflow: hidden;
  }
  .editor :global(.cm-editor) {
    height: 100%;
  }
  .divider {
    flex: none;
    height: 5px;
    cursor: row-resize;
    border-top: 1px solid var(--border);
    background: var(--bg);
  }
  .results {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .rtabs {
    display: flex;
    gap: 2px;
    padding: 4px 6px 0;
    background: var(--bg);
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
  }
  .rtabs button {
    border: 1px solid transparent;
    border-bottom: 0;
    background: none;
    padding: 3px 10px;
    border-radius: 5px 5px 0 0;
    color: var(--muted);
    white-space: nowrap;
  }
  .rtabs button.on {
    background: var(--panel);
    border-color: var(--border);
    color: var(--text);
  }
  .placeholder {
    flex: 1;
    display: grid;
    place-items: center;
    padding: 16px;
    text-align: center;
  }
  .message {
    padding: 12px 14px;
    overflow: auto;
  }
  .message p {
    margin: 0 0 8px;
  }
  .message pre {
    margin: 0;
    font-family: var(--mono);
    font-size: 12px;
    white-space: pre-wrap;
  }
  .bar {
    flex: none;
    display: flex;
    gap: 12px;
    align-items: center;
    height: 26px;
    padding: 0 10px;
    font-size: 12px;
    border-top: 1px solid var(--border);
    background: var(--bg);
  }
  .warn {
    color: #b45309;
  }
</style>
