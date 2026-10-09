<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { app, queryModified } from "../lib/state/app.svelte";
  import { isMac } from "../lib/api";
  import { kindLabel } from "../lib/cells";
  import ConnectionRail from "./ConnectionRail.svelte";
  import TableView from "./TableView.svelte";
  import QueryEditor from "./QueryEditor.svelte";
  import CollectionView from "./CollectionView.svelte";
  import MongoCommand from "./MongoCommand.svelte";
  import RedisWorkspace from "./RedisWorkspace.svelte";

  const session = $derived(app.session);
  const queryLabel = $derived(session?.kind === "mongodb" ? "Command" : "SQL");
  const sqlFiles = $derived(!!session && session.kind !== "redis" && session.kind !== "mongodb");

  // Query tabs survive a disconnect or restart: keep them saved as they change.
  $effect(() => app.persistQueries());

  const RAIL_KEY = "tablory.connectionRail";
  let showRail = $state(readRail());
  function readRail() {
    try {
      return localStorage.getItem(RAIL_KEY) !== "hidden";
    } catch {
      return true;
    }
  }
  function toggleRail() {
    showRail = !showRail;
    try {
      localStorage.setItem(RAIL_KEY, showRail ? "shown" : "hidden");
    } catch {}
  }

  /** Runs a File/View menu command, also reached through its shortcut. */
  function command(id: string) {
    if (app.confirm) return;
    const tab = app.tabs.find((t) => t.id === app.activeTab);
    switch (id) {
      case "new-query":
        if (session && session.kind !== "redis") app.newQuery();
        break;
      case "open-file":
        if (sqlFiles) app.openQueryFile();
        break;
      case "save":
      case "save-as":
        if (tab?.kind === "query") app.saveQuery(tab, id === "save-as");
        break;
      case "close-tab":
        if (app.activeTab) app.closeTab(app.activeTab);
        break;
      case "toggle-rail":
        toggleRail();
        break;
      case "disconnect":
        app.disconnect();
        break;
      case "manage":
        app.view = "connections";
        break;
    }
  }

  onMount(() => {
    const off = listen<string>("menu", (e) => command(e.payload));
    return () => off.then((f) => f());
  });

  const KEYS: Record<string, string> = { t: "new-query", o: "open-file", w: "close-tab", b: "toggle-rail", k: "disconnect" };

  function onkeydown(e: KeyboardEvent) {
    const mod = e.metaKey || e.ctrlKey;
    if (!mod || app.confirm || e.shiftKey || e.altKey) return;
    const id = KEYS[e.key];
    if (!id) return;
    e.preventDefault();
    command(id);
  }
</script>

<svelte:window {onkeydown} />

<div class="ws">
  <header class="toolbar" class:mac={isMac} data-tauri-drag-region>
    <button
      class="btn ghost rail-toggle"
      class:on={showRail}
      onclick={toggleRail}
      title="{showRail ? 'Hide' : 'Show'} connections (⌘B)"
      aria-label="{showRail ? 'Hide' : 'Show'} connections"
      aria-pressed={showRail}>☰</button
    >
    {#if session}
      <span class="conn" data-tauri-drag-region>
        <span class="swatch" style:background={session.color || "var(--muted)"}></span>
        <strong>{session.name}</strong>
        <span class="muted">{kindLabel[session.kind]}</span>
      </span>
      {#if session.database}
        <span class="muted db" data-tauri-drag-region>/ {session.kind === "redis" ? `db ${session.database}` : session.database}</span>
      {/if}
    {:else}
      <span class="muted" data-tauri-drag-region>Not connected</span>
    {/if}
    <span class="spacer" data-tauri-drag-region></span>
    {#if session && session.kind !== "redis"}
      <button class="btn" onclick={() => app.newQuery()} title="New {queryLabel.toLowerCase()} tab (⌘T)">{queryLabel}</button>
    {/if}
    {#if sqlFiles}
      <button class="btn" onclick={() => app.openQueryFile()} title="Open a .sql file (⌘O)">Open…</button>
    {/if}
    {#if session}
      <button class="btn" onclick={() => app.disconnect()} title="Disconnect (⌘K)">Disconnect</button>
    {/if}
    <button class="btn" onclick={() => (app.view = "connections")} title="Add, edit or import connections">Manage…</button>
  </header>

  <div class="frame" class:with-rail={showRail}>
  {#if showRail}<ConnectionRail />{/if}
  <div class="content">
  {#if !session}
    <div class="empty">
      <p>Not connected. Pick a connection on the left.</p>
      {#if !showRail}<button class="btn" onclick={toggleRail}>Show connections <span class="kbd">⌘B</span></button>{/if}
    </div>
  {:else}
  {#key session.id}
  {#if session.kind === "redis"}
    {#key session.database}<RedisWorkspace />{/key}
  {:else}
  <div class="body">
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
                title={t.kind === "query" && t.path ? t.path : undefined}
              >
                {#if t.dirty}<span class="dirty" title="Unsaved changes">●</span>{/if}
                <span class="kind-tag">{t.kind === "query" ? "SQL" : t.kind === "mongo-command" ? "CMD" : ""}</span>{t.title}{#if t.kind === "query" && queryModified(t)}<span class="unsaved" title="Not saved">*</span>{/if}
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
  {/key}
  {/if}
  </div>
  </div>
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
    max-width: 240px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .spacer {
    flex: 1;
    align-self: stretch;
  }
  .frame {
    flex: 1;
    display: grid;
    grid-template-columns: 1fr;
    min-height: 0;
  }
  .frame.with-rail {
    grid-template-columns: 260px 1fr;
  }
  .content {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }
  .rail-toggle {
    padding: 0 7px;
    color: var(--muted);
  }
  .rail-toggle.on {
    color: var(--text);
  }
  .body {
    flex: 1;
    display: flex;
    min-height: 0;
  }
  .main {
    flex: 1;
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
  .unsaved {
    margin-left: 1px;
    color: #d97706;
    font-weight: 600;
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
