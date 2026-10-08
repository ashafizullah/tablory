<script lang="ts">
  import { app } from "../lib/state/app.svelte";
  import { isMac } from "../lib/api";
  import { kindLabel } from "../lib/cells";
  import Sidebar from "./Sidebar.svelte";
  import TableView from "./TableView.svelte";
  import QueryEditor from "./QueryEditor.svelte";
  import CollectionView from "./CollectionView.svelte";
  import MongoCommand from "./MongoCommand.svelte";
  import RedisWorkspace from "./RedisWorkspace.svelte";

  const session = $derived(app.session!);
  const queryLabel = $derived(session.kind === "mongodb" ? "Command" : "SQL");

  function onkeydown(e: KeyboardEvent) {
    const mod = e.metaKey || e.ctrlKey;
    if (!mod || app.confirm) return;
    if (e.key === "t" && session.kind !== "redis") {
      e.preventDefault();
      app.newQuery();
    } else if (e.key === "w") {
      e.preventDefault();
      if (app.activeTab) app.closeTab(app.activeTab);
    } else if (e.key === "k") {
      e.preventDefault();
      app.disconnect();
    }
  }
</script>

<svelte:window {onkeydown} />

<div class="ws">
  <header class="toolbar" class:mac={isMac} data-tauri-drag-region>
    <span class="conn" data-tauri-drag-region>
      <span class="swatch" style:background={session.color || "var(--muted)"}></span>
      <strong>{session.name}</strong>
      <span class="muted">{kindLabel[session.kind]}</span>
    </span>
    {#if (session.kind === "postgres" || session.kind === "mssql") && app.databases.length > 0}
      <select
        class="field db"
        aria-label="Database"
        value={session.database}
        onchange={(e) => app.switchDatabase(e.currentTarget.value)}
      >
        {#each app.databases as db}<option value={db}>{db}</option>{/each}
      </select>
    {:else if session.kind === "redis" && app.databases.length > 0}
      <select
        class="field db"
        aria-label="Database"
        value={session.database}
        onchange={(e) => app.switchDatabase(e.currentTarget.value)}
      >
        {#each app.databases as db}<option value={db}>db {db}</option>{/each}
      </select>
    {/if}
    <span class="spacer" data-tauri-drag-region></span>
    {#if session.kind !== "redis"}
      <button class="btn" onclick={() => app.newQuery()} title="New {queryLabel.toLowerCase()} tab (⌘T)">{queryLabel}</button>
    {/if}
    <button class="btn" onclick={() => app.disconnect()} title="Back to connections (⌘K)">Disconnect</button>
  </header>

  {#if session.kind === "redis"}
    {#key session.database}<RedisWorkspace />{/key}
  {:else}
  <div class="body">
    <Sidebar />
    <main class="main">
      {#if app.tabs.length > 0}
        <div class="tabs" role="tablist">
          {#each app.tabs as t (t.id)}
            <div class="tab" class:active={t.id === app.activeTab} role="presentation">
              <button
                class="tab-label"
                role="tab"
                aria-selected={t.id === app.activeTab}
                onclick={() => (app.activeTab = t.id)}
                onauxclick={(e) => e.button === 1 && app.closeTab(t.id)}
              >
                {#if t.dirty}<span class="dirty" title="Unsaved changes">●</span>{/if}
                <span class="kind-tag">{t.kind === "query" ? "SQL" : t.kind === "mongo-command" ? "CMD" : ""}</span>{t.title}
              </button>
              <button class="close" aria-label="Close {t.title}" onclick={() => app.closeTab(t.id)}>×</button>
            </div>
          {/each}
        </div>
      {/if}
      <div class="panes">
        {#each app.tabs as t (t.id)}
          <div class="pane" hidden={t.id !== app.activeTab}>
            {#if t.kind === "table"}
              <TableView tab={t} active={t.id === app.activeTab} />
            {:else if t.kind === "query"}
              <QueryEditor tab={t} active={t.id === app.activeTab} />
            {:else if t.kind === "collection"}
              <CollectionView tab={t} active={t.id === app.activeTab} />
            {:else}
              <MongoCommand tab={t} active={t.id === app.activeTab} />
            {/if}
          </div>
        {/each}
        {#if app.tabs.length === 0}
          <div class="empty">
            {#if session.kind === "mongodb"}
              <p>Open a collection from the sidebar, or run a database command.</p>
              <button class="btn" onclick={() => app.newQuery()}>New command <span class="kbd">⌘T</span></button>
            {:else}
              <p>Open a table from the sidebar, or start a query.</p>
              <button class="btn" onclick={() => app.newQuery()}>New SQL query <span class="kbd">⌘T</span></button>
            {/if}
          </div>
        {/if}
      </div>
    </main>
  </div>
  {/if}
</div>

<style>
  .ws {
    display: flex;
    flex-direction: column;
    height: 100vh;
  }
  .toolbar {
    height: 40px;
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 10px;
    border-bottom: 1px solid var(--border);
    background: var(--bg);
  }
  .toolbar.mac {
    padding-left: 84px;
  }
  .conn {
    display: flex;
    align-items: center;
    gap: 7px;
    white-space: nowrap;
  }
  .swatch {
    width: 10px;
    height: 10px;
    border-radius: 3px;
  }
  .db {
    max-width: 200px;
  }
  .spacer {
    flex: 1;
    align-self: stretch;
  }
  .body {
    flex: 1;
    display: grid;
    grid-template-columns: 240px 1fr;
    min-height: 0;
  }
  .main {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    background: var(--panel);
  }
  .tabs {
    display: flex;
    flex: none;
    overflow-x: auto;
    border-bottom: 1px solid var(--border);
    background: var(--bg);
  }
  .tab {
    display: flex;
    align-items: center;
    border-right: 1px solid var(--border);
    max-width: 220px;
  }
  .tab.active {
    background: var(--panel);
  }
  .tab-label {
    border: 0;
    background: none;
    padding: 6px 4px 6px 12px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--muted);
  }
  .tab.active .tab-label {
    color: var(--text);
  }
  .kind-tag {
    font-size: 10px;
    font-weight: 600;
    margin-right: 4px;
    color: var(--muted);
  }
  .dirty {
    color: #d97706;
    font-size: 9px;
    margin-right: 4px;
  }
  .close {
    border: 0;
    background: none;
    color: var(--muted);
    width: 22px;
    height: 22px;
    border-radius: 4px;
    margin-right: 4px;
    font-size: 15px;
    line-height: 1;
  }
  .close:hover {
    background: var(--hover);
    color: var(--text);
  }
  .panes {
    flex: 1;
    position: relative;
    min-height: 0;
  }
  .pane {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
  }
  .pane[hidden] {
    display: none;
  }
  .empty {
    height: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
    color: var(--muted);
  }
</style>
