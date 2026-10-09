<script lang="ts">
  import { app } from "../lib/state/app.svelte";
  import { kindLabel } from "../lib/cells";
  import type { ConnectionProfile, RoutineInfo, TableInfo } from "../lib/types";

  // One tree: connection → database → object categories → objects. Only the
  // current connection and its current database are expanded with content.

  let search = $state("");
  let searchInput: HTMLInputElement | undefined = $state();
  const q = $derived(search.trim().toLowerCase());
  const hit = (name: string) => !q || name.toLowerCase().includes(q);

  function load(key: string): string[] | null {
    try {
      return JSON.parse(localStorage.getItem(key) ?? "null");
    } catch {
      return null;
    }
  }
  function save(key: string, v: string[]) {
    try {
      localStorage.setItem(key, JSON.stringify(v));
    } catch {}
  }

  // Groups share their collapsed state with the connections screen.
  const GROUPS_KEY = "tablory.collapsedGroups";
  let collapsedGroups = $state<string[]>(load(GROUPS_KEY) ?? []);
  function toggleGroup(g: string) {
    collapsedGroups = collapsedGroups.includes(g) ? collapsedGroups.filter((x) => x !== g) : [...collapsedGroups, g];
    save(GROUPS_KEY, collapsedGroups);
  }

  // Object categories start collapsed except tables.
  const CATS_KEY = "tablory.openCategories";
  let openCats = $state<string[]>(load(CATS_KEY) ?? ["table"]);
  function toggleCat(c: string) {
    openCats = openCats.includes(c) ? openCats.filter((x) => x !== c) : [...openCats, c];
    save(CATS_KEY, openCats);
  }

  let expanded = $state(true);
  let dbOpen = $state(true);

  const session = $derived(app.session);
  const kind = $derived(session?.kind);
  /** MySQL and MongoDB schemas are databases. */
  const schemasAreDbs = $derived(kind === "mysql" || kind === "mongodb");
  const dbs = $derived(kind === "sqlite" ? [] : schemasAreDbs ? app.schemas : app.databases);
  const currentDb = $derived(schemasAreDbs ? app.schema : (session?.database ?? ""));
  const showSchemas = $derived(!schemasAreDbs && app.schemas.length > 1);

  type Cat = { key: string; label: string; tables?: TableInfo[]; routines?: RoutineInfo[] };
  const categories = $derived.by((): Cat[] => {
    if (kind === "redis") return [];
    const t = app.tables.filter((x) => hit(x.name));
    const r = app.routines.filter((x) => hit(x.name));
    if (kind === "mongodb") return [{ key: "table", label: "Collections", tables: t }];
    const cats: Cat[] = [
      { key: "table", label: "Tables", tables: t.filter((x) => x.kind === "table") },
      { key: "view", label: "Views", tables: t.filter((x) => x.kind === "view") },
    ];
    if (kind !== "sqlite") {
      cats.push(
        { key: "function", label: "Functions", routines: r.filter((x) => x.kind === "function") },
        { key: "procedure", label: "Procedures", routines: r.filter((x) => x.kind === "procedure") },
      );
    }
    return cats;
  });
  const firstTable = $derived(categories.flatMap((c) => c.tables ?? [])[0]);

  const activeTable = $derived.by(() => {
    const t = app.tabs.find((t) => t.id === app.activeTab);
    if (t?.kind === "table" && t.table.schema === app.schema) return t.table.name;
    if (t?.kind === "collection" && t.db === app.schema) return t.name;
    return null;
  });

  // The current connection always shows, so its objects stay searchable.
  const sections = $derived.by(() => {
    const show = (c: ConnectionProfile) => c.id === session?.connection_id || hit(c.name || "");
    const of = (g: string) => app.connections.filter((c) => c.group === g && show(c));
    const groups = app.groups.map((g) => ({ group: g, list: of(g) }));
    return [{ group: "", list: of("") }, ...(q ? groups.filter((s) => s.list.length > 0) : groups)];
  });

  function openConnection(c: ConnectionProfile) {
    if (c.id === session?.connection_id) {
      expanded = !expanded;
    } else {
      expanded = true;
      dbOpen = true;
      app.connect(c.id);
    }
  }

  function openDb(db: string) {
    if (db === currentDb) {
      dbOpen = !dbOpen;
      return;
    }
    dbOpen = true;
    if (schemasAreDbs) app.selectSchema(db);
    else app.switchDatabase(db);
  }

  function onkeydown(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key === "f" && !e.shiftKey && !app.confirm) {
      e.preventDefault();
      searchInput?.focus();
    }
  }

  function searchKey(e: KeyboardEvent) {
    if (e.key === "Enter" && firstTable) app.openTable(firstTable.name);
    if (e.key === "Escape") search = "";
  }

  const pad = (depth: number) => `${6 + depth * 14}px`;
</script>

<svelte:window {onkeydown} />

{#snippet objects(depth: number)}
  {#if app.sidebarError}
    <p class="note error-text" style:padding-left={pad(depth)}>
      {app.sidebarError}
      <button class="link" onclick={() => app.loadSidebar()}>Retry</button>
    </p>
  {:else if app.tablesLoading && app.tables.length === 0}
    <p class="note muted" style:padding-left={pad(depth)}>Loading…</p>
  {:else}
    {#if showSchemas}
      <div class="schema" style:padding-left={pad(depth)}>
        <select
          class="field"
          aria-label="Schema"
          value={app.schema}
          onchange={(e) => app.selectSchema(e.currentTarget.value)}
        >
          {#each app.schemas as s}<option value={s}>{s}</option>{/each}
        </select>
      </div>
    {/if}
    {#each categories as cat (cat.key)}
      {@const items = cat.tables ?? cat.routines ?? []}
      {@const isOpen = !!q || openCats.includes(cat.key)}
      {#if !q || items.length > 0}
        <button class="row cat" style:padding-left={pad(depth)} aria-expanded={isOpen} onclick={() => toggleCat(cat.key)}>
          <span class="chevron" class:closed={!isOpen}>▾</span>
          <span class="name">{cat.label}</span>
          <span class="muted count">{items.length}</span>
        </button>
        {#if isOpen}
          {#each cat.tables ?? [] as t (t.name)}
            <button
              class="row"
              class:on={t.name === activeTable}
              style:padding-left={pad(depth + 1)}
              onclick={() => app.openTable(t.name)}
              title={t.name}
            >
              <span class="icon" aria-hidden="true">{t.kind === "view" ? "◇" : "▦"}</span>
              <span class="name">{t.name}</span>
            </button>
          {/each}
          {#each cat.routines ?? [] as r (r.id)}
            <button
              class="row"
              style:padding-left={pad(depth + 1)}
              onclick={() => app.openRoutine(r)}
              title="{r.name} — open definition"
            >
              <span class="icon" aria-hidden="true">ƒ</span>
              <span class="name">{r.name}</span>
            </button>
          {/each}
          {#if items.length === 0}
            <p class="note muted" style:padding-left={pad(depth + 1)}>None</p>
          {/if}
        {/if}
      {/if}
    {/each}
  {/if}
{/snippet}

{#snippet connection(c: ConnectionProfile, depth: number)}
  {@const current = c.id === session?.connection_id}
  <button
    class="row conn"
    class:current
    style:padding-left={pad(depth)}
    onclick={() => openConnection(c)}
    aria-expanded={current ? expanded : undefined}
    title="{c.name || 'Untitled'} · {kindLabel[c.kind]}"
  >
    <span class="chevron" class:closed={!current || !expanded}>▾</span>
    <span class="swatch" style:background={c.color || "var(--muted)"}></span>
    <span class="name">{c.name || "Untitled"}</span>
    {#if app.connecting === c.id}
      <span class="muted">…</span>
    {:else if current}
      <span class="live" title="Connected" aria-label="Connected"></span>
    {/if}
  </button>
  {#if current && expanded}
    {#if kind === "sqlite"}
      {@render objects(depth + 1)}
    {:else}
      {#each dbs as db (db)}
        {@const on = db === currentDb}
        <button
          class="row db"
          class:on={on && kind === "redis"}
          style:padding-left={pad(depth + 1)}
          onclick={() => openDb(db)}
          aria-expanded={on && kind !== "redis" ? dbOpen : undefined}
          title={db}
        >
          {#if kind !== "redis"}<span class="chevron" class:closed={!on || !dbOpen}>▾</span>{/if}
          <span class="icon" class:active={on} aria-hidden="true">⛁</span>
          <span class="name" class:strong={on}>{kind === "redis" ? `db ${db}` : db}</span>
        </button>
        {#if on && dbOpen && kind !== "redis"}
          {@render objects(depth + 2)}
        {/if}
      {:else}
        <p class="note muted" style:padding-left={pad(depth + 1)}>{app.connecting ? "Loading…" : "No databases"}</p>
      {/each}
    {/if}
  {/if}
{/snippet}

<nav class="rail" aria-label="Connections">
  <div class="top">
    <input
      class="field search"
      placeholder="Search"
      aria-label="Search connections and objects"
      bind:value={search}
      bind:this={searchInput}
      onkeydown={searchKey}
    />
    <button class="btn ghost refresh" title="Reload objects" aria-label="Reload objects" onclick={() => app.loadSidebar()}
      >↻</button
    >
  </div>
  <div class="items">
    {#each sections[0].list as c (c.id)}
      {@render connection(c, 0)}
    {/each}
    {#each sections.slice(1) as { group: g, list } (g)}
      {@const isOpen = !!q || !collapsedGroups.includes(g)}
      <button class="row group" aria-expanded={isOpen} onclick={() => toggleGroup(g)}>
        <span class="chevron" class:closed={!isOpen}>▾</span>
        {#if app.groupColors[g]}<span class="folder" style:background={app.groupColors[g]}></span>{/if}
        <span class="name label" style:color={app.groupColors[g]}>{g}</span>
        <span class="muted count">{list.length}</span>
      </button>
      {#if isOpen}
        {#each list as c (c.id)}
          {@render connection(c, 1)}
        {/each}
      {/if}
    {/each}
  </div>
  {#if app.connectError}
    <p class="error-text err">
      {app.connectError}
      <button class="link muted" onclick={() => (app.connectError = null)} title="Dismiss">✕</button>
    </p>
  {/if}
</nav>

<style>
  .rail {
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--sidebar);
    border-right: 1px solid var(--border);
  }
  .top {
    display: flex;
    gap: 4px;
    padding: 8px;
  }
  .search {
    flex: 1;
    min-width: 0;
  }
  .refresh {
    padding: 0 6px;
    color: var(--muted);
  }
  .items {
    flex: 1;
    overflow: auto;
    padding: 0 6px 10px;
    user-select: none;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    height: 24px;
    padding-right: 6px;
    border: 0;
    border-radius: 5px;
    background: none;
    text-align: left;
  }
  .row:hover {
    background: var(--hover);
  }
  .row.on {
    background: var(--selection);
  }
  .conn {
    height: 26px;
  }
  .conn.current .name,
  .strong {
    font-weight: 600;
  }
  .group {
    margin-top: 6px;
    gap: 4px;
    font-size: 11px;
    font-weight: 600;
    color: var(--muted);
  }
  .label {
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }
  .cat {
    color: var(--muted);
    font-size: 12px;
  }
  .count {
    font-weight: 400;
    font-size: 11px;
  }
  .chevron {
    display: inline-block;
    width: 10px;
    flex: none;
    color: var(--muted);
    font-size: 11px;
    transition: transform 0.1s;
  }
  .chevron.closed {
    transform: rotate(-90deg);
  }
  .icon {
    width: 12px;
    flex: none;
    text-align: center;
    color: var(--muted);
    font-size: 11px;
  }
  .icon.active {
    color: var(--accent);
  }
  .name {
    flex: 1;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .live {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex: none;
    background: #22c55e;
    box-shadow: 0 0 0 2px color-mix(in srgb, #22c55e 25%, transparent);
  }
  .swatch {
    width: 9px;
    height: 9px;
    border-radius: 3px;
    flex: none;
  }
  .folder {
    width: 8px;
    height: 8px;
    border-radius: 2px;
    flex: none;
  }
  .schema {
    padding: 2px 6px 4px 0;
  }
  .schema .field {
    width: 100%;
    height: 24px;
    font-size: 12px;
  }
  .note {
    margin: 0;
    padding: 3px 6px;
    font-size: 12px;
    word-break: break-word;
  }
  .err {
    margin: 0;
    padding: 8px 10px;
    font-size: 12px;
    border-top: 1px solid var(--border);
    word-break: break-word;
  }
  .link {
    border: 0;
    background: none;
    font-size: 11px;
    padding: 0 4px;
    text-decoration: underline;
  }
</style>
