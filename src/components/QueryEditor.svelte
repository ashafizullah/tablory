<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { basicSetup } from "codemirror";
  import { Compartment, EditorState, Prec } from "@codemirror/state";
  import { EditorView } from "@codemirror/view";
  import { sql as sqlLang } from "@codemirror/lang-sql";
  import { app, type Tab } from "../lib/state/app.svelte";
  import { api, errorText } from "../lib/api";
  import { uid } from "../lib/cells";
  import { dialectFor, highlight, keymap, theme } from "../lib/codemirror";
  import { splitStatements, statementAt } from "../lib/sqlsplit";
  import type { ExecuteResult } from "../lib/types";
  import DataGrid from "./DataGrid.svelte";

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

  const session = $derived(app.session!);
  const mysql = $derived(session.kind === "mysql");

  function languageExt() {
    const schema: Record<string, string[]> = {};
    for (const t of app.tables) schema[t.name] = [];
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
    const qid = uid();
    running = qid;
    error = null;
    ranSql = sql;
    try {
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
  }

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

  // Keep autocomplete in sync with the table list.
  $effect(() => {
    void app.tables;
    view?.dispatch({ effects: language.reconfigure(languageExt()) });
  });

  $effect(() => {
    if (active) queueMicrotask(() => view?.focus());
  });

  onDestroy(() => view?.destroy());

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

<div class="qe">
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
    <span class="grow"></span>
    <label class="muted small" for="max-{tab.id}">Row limit</label>
    <select id="max-{tab.id}" class="field" bind:value={maxRows}>
      <option value={200}>200</option>
      <option value={1000}>1,000</option>
      <option value={5000}>5,000</option>
      <option value={20000}>20,000</option>
    </select>
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
    <span>{session.database}</span>
  </footer>
</div>

<style>
  .qe {
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
