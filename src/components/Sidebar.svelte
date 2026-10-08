<script lang="ts">
  import { app } from "../lib/state/app.svelte";

  let search = $state("");
  const mongo = $derived(app.session?.kind === "mongodb");
  const noun = $derived(mongo ? "collections" : "tables");
  let searchInput: HTMLInputElement | undefined = $state();

  const filtered = $derived.by(() => {
    const q = search.trim().toLowerCase();
    return q ? app.tables.filter((t) => t.name.toLowerCase().includes(q)) : app.tables;
  });
  const tables = $derived(filtered.filter((t) => t.kind === "table"));
  const views = $derived(filtered.filter((t) => t.kind === "view"));
  const activeName = $derived.by(() => {
    const t = app.tabs.find((t) => t.id === app.activeTab);
    return t?.kind === "table" && t.table.schema === app.schema ? t.table.name : null;
  });

  function onkeydown(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key === "f" && !e.shiftKey) {
      e.preventDefault();
      searchInput?.focus();
    }
  }

  function searchKey(e: KeyboardEvent) {
    if (e.key === "Enter" && filtered.length > 0) app.openTable(filtered[0].name);
    if (e.key === "Escape") search = "";
  }
</script>

<svelte:window {onkeydown} />

<aside class="sidebar">
  <div class="top">
    {#if app.schemas.length > 1 || app.session?.kind === "mysql" || mongo}
      <select
        class="field"
        aria-label={app.session?.kind === "mysql" || mongo ? "Database" : "Schema"}
        value={app.schema}
        onchange={(e) => app.selectSchema(e.currentTarget.value)}
      >
        {#each app.schemas as s}<option value={s}>{s}</option>{/each}
      </select>
    {/if}
    <div class="search-row">
      <input
        class="field search"
        placeholder="Search {noun}"
        aria-label="Search {noun}"
        bind:value={search}
        bind:this={searchInput}
        onkeydown={searchKey}
      />
      <button class="btn ghost refresh" title="Reload {noun}" aria-label="Reload {noun}" onclick={() => app.loadSidebar()}
        >↻</button
      >
    </div>
  </div>

  <div class="list">
    {#if app.sidebarError}
      <p class="error-text pad">{app.sidebarError}</p>
      <button class="btn retry" onclick={() => app.loadSidebar()}>Retry</button>
    {:else if app.tablesLoading && app.tables.length === 0}
      <p class="muted pad">Loading {noun}…</p>
    {:else if app.tables.length === 0}
      <p class="muted pad">
        No {noun} in {app.schema || "this database"}.
        {mongo ? "Insert a document with a command (⌘T) to create one." : "Create one from a SQL query (⌘T)."}
      </p>
    {:else if filtered.length === 0}
      <p class="muted pad">No {noun.slice(0, -1)} matches “{search}”.</p>
    {/if}

    {#each [{ label: mongo ? "Collections" : "Tables", items: tables }, { label: "Views", items: views }] as group}
      {#if group.items.length > 0}
        <h2>{group.label} <span class="muted">{group.items.length}</span></h2>
        {#each group.items as t (t.name)}
          <button class="item" class:active={t.name === activeName} onclick={() => app.openTable(t.name)} title={t.name}>
            <span class="icon" aria-hidden="true">{t.kind === "view" ? "◇" : "▦"}</span>
            <span class="name">{t.name}</span>
          </button>
        {/each}
      {/if}
    {/each}
  </div>
</aside>

<style>
  .sidebar {
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--sidebar);
    border-right: 1px solid var(--border);
  }
  .top {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px;
  }
  .search-row {
    display: flex;
    gap: 4px;
  }
  .search {
    flex: 1;
  }
  .refresh {
    padding: 0 6px;
    color: var(--muted);
  }
  .list {
    flex: 1;
    overflow: auto;
    padding: 0 6px 10px;
  }
  h2 {
    font-size: 11px;
    font-weight: 600;
    color: var(--muted);
    margin: 10px 6px 4px;
  }
  .item {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    height: 24px;
    padding: 0 6px;
    border: 0;
    border-radius: 5px;
    background: none;
    text-align: left;
  }
  .item:hover {
    background: var(--hover);
  }
  .item.active {
    background: var(--selection);
  }
  .icon {
    color: var(--muted);
    font-size: 11px;
    width: 12px;
  }
  .name {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .pad {
    padding: 6px;
    margin: 0;
  }
  .retry {
    margin-left: 6px;
  }
</style>
