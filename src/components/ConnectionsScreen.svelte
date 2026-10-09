<script lang="ts">
  import { app } from "../lib/state/app.svelte";
  import { onMount } from "svelte";
  import { api, errorText, invoke, isMac } from "../lib/api";
  import { kindLabel } from "../lib/cells";
  import type { ConnectionProfile } from "../lib/types";
  import ConnectionForm from "./ConnectionForm.svelte";

  let { loadError }: { loadError: string | null } = $props();

  // null = new connection form.
  let selectedId = $state<string | null>(null);
  let formKey = $state(0);
  let version = $state("");
  // Group of the next new connection.
  let newGroup = $state("");
  let notice = $state<{ ok: boolean; text: string; detail: string[] } | null>(null);

  onMount(() => {
    invoke<string>("app_version").then((v) => (version = v)).catch(() => {});
  });
  let selected = $derived(app.connections.find((c) => c.id === selectedId) ?? null);

  $effect(() => {
    if (selectedId === null && app.connections.length > 0 && formKey === 0) selectedId = app.connections[0].id;
  });

  // Ungrouped connections first (no header), then groups in saved order.
  let sections = $derived.by(() => {
    const of = (g: string) => app.connections.filter((c) => c.group === g);
    return [{ group: "", list: of("") }, ...app.groups.map((g) => ({ group: g, list: of(g) }))];
  });

  const COLLAPSED_KEY = "tablory.collapsedGroups";
  let collapsed = $state<string[]>(readCollapsed());
  function readCollapsed(): string[] {
    try {
      return JSON.parse(localStorage.getItem(COLLAPSED_KEY) ?? "[]");
    } catch {
      return [];
    }
  }
  function setCollapsed(next: string[]) {
    collapsed = next;
    try {
      localStorage.setItem(COLLAPSED_KEY, JSON.stringify(next));
    } catch {}
  }
  const toggle = (g: string) =>
    setCollapsed(collapsed.includes(g) ? collapsed.filter((x) => x !== g) : [...collapsed, g]);

  function subtitle(c: ConnectionProfile) {
    if (c.kind === "sqlite") return c.file.split(/[\\/]/).pop() || "SQLite";
    const host = `${c.host || "localhost"}${c.port ? ":" + c.port : ""}`;
    return c.ssh.enabled ? `${host} via ${c.ssh.host}` : host;
  }

  function newConnection(group = "") {
    newGroup = group;
    selectedId = null;
    formKey += 1;
  }

  /** Runs a list change, reloads, and shows any error under the list. */
  async function run(f: () => Promise<unknown>) {
    try {
      await f();
      await app.loadConnections();
    } catch (e) {
      notice = { ok: false, text: errorText(e), detail: [] };
    }
  }

  async function importNavicat() {
    notice = null;
    try {
      const r = await api.importNavicat();
      if (!r) return;
      await app.loadConnections();
      const n = r.imported;
      notice = {
        ok: n > 0 || r.skipped.length === 0,
        text: `Imported ${n} connection${n === 1 ? "" : "s"}` + (r.skipped.length ? `, skipped ${r.skipped.length}` : ""),
        detail: r.skipped,
      };
    } catch (e) {
      notice = { ok: false, text: errorText(e), detail: [] };
    }
  }

  // ---- Groups: create and rename inline ----

  /** New-group input; `moveId` is a connection to put into the new group. */
  let creating = $state<{ moveId: string | null } | null>(null);
  let renaming = $state<string | null>(null);

  function startNewGroup(moveId: string | null = null) {
    renaming = null;
    creating = { moveId };
  }

  async function finishNewGroup(name: string) {
    const c = creating;
    creating = null;
    if (!c || !name.trim()) return;
    await run(async () => {
      const g = await api.createGroup(name);
      if (c.moveId) await api.moveConnection(c.moveId, g);
    });
  }

  async function finishRename(from: string, to: string) {
    renaming = null;
    to = to.trim();
    if (!to || to === from) return;
    await run(() => api.renameGroup(from, to));
    if (collapsed.includes(from)) setCollapsed(collapsed.map((g) => (g === from ? to : g)));
  }

  function focus(el: HTMLInputElement) {
    el.focus();
    el.select();
  }

  function inputKeys(e: KeyboardEvent & { currentTarget: HTMLInputElement }, cancel: () => void) {
    if (e.key === "Enter") e.currentTarget.blur();
    if (e.key === "Escape") cancel();
  }

  async function deleteConnection(c: ConnectionProfile) {
    const ok = await app.ask(`Delete “${c.name}”?`, {
      detail: "The saved password is removed from the keychain too.",
      ok: "Delete",
      danger: true,
    });
    if (!ok) return;
    await run(() => api.deleteConnection(c.id));
    if (selectedId === c.id) {
      selectedId = app.connections[0]?.id ?? null;
      formKey += 1;
    }
  }

  // ---- Right-click menu ----

  type MenuItem = { label: string; action: () => void; danger?: boolean } | "sep";
  let menu = $state<{ x: number; y: number; items: MenuItem[] } | null>(null);

  function openMenu(e: MouseEvent, items: MenuItem[]) {
    e.preventDefault();
    e.stopPropagation();
    const h = items.reduce((n, i) => n + (i === "sep" ? 9 : 26), 10);
    menu = {
      x: Math.min(e.clientX, window.innerWidth - 210),
      y: Math.max(4, Math.min(e.clientY, window.innerHeight - h - 4)),
      items,
    };
  }

  function listMenu(e: MouseEvent) {
    openMenu(e, [
      { label: "New connection", action: () => newConnection() },
      { label: "New group", action: () => startNewGroup() },
      "sep",
      { label: "Import from Navicat…", action: importNavicat },
    ]);
  }

  function groupMenu(e: MouseEvent, g: string) {
    openMenu(e, [
      { label: "New connection here", action: () => newConnection(g) },
      { label: "Rename group", action: () => ((creating = null), (renaming = g)) },
      { label: "Delete group", danger: true, action: () => run(() => api.deleteGroup(g)) },
      "sep",
      { label: "New group", action: () => startNewGroup() },
    ]);
  }

  function connectionMenu(e: MouseEvent, c: ConnectionProfile) {
    selectedId = c.id;
    const moves: MenuItem[] = app.groups
      .filter((g) => g !== c.group)
      .map((g) => ({ label: `Move to ${g}`, action: () => run(() => api.moveConnection(c.id, g)) }));
    if (c.group) moves.push({ label: "Remove from group", action: () => run(() => api.moveConnection(c.id, "")) });
    openMenu(e, [
      { label: "Connect", action: () => app.connect(c.id) },
      "sep",
      ...moves,
      { label: "New group with this connection", action: () => startNewGroup(c.id) },
      "sep",
      { label: "Delete connection", danger: true, action: () => deleteConnection(c) },
    ]);
  }

  // ---- Drag and drop ----
  // Pointer events rather than HTML5 drag and drop, which the Windows webview
  // swallows while Tauri handles file drops.

  let drag = $state<{ id: string; name: string; from: string; x: number; y: number; target: string | null } | null>(
    null,
  );
  let dragged = false;

  function startDrag(e: PointerEvent, c: ConnectionProfile) {
    if (e.button !== 0) return;
    const sx = e.clientX;
    const sy = e.clientY;
    let active = false;
    const move = (ev: PointerEvent) => {
      if (!active && Math.hypot(ev.clientX - sx, ev.clientY - sy) < 5) return;
      active = true;
      const el = document.elementFromPoint(ev.clientX, ev.clientY)?.closest<HTMLElement>("[data-drop]");
      drag = {
        id: c.id,
        name: c.name || "Untitled",
        from: c.group,
        x: ev.clientX,
        y: ev.clientY,
        target: el ? (el.dataset.drop ?? null) : null,
      };
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      const d = drag;
      drag = null;
      if (!active) return;
      // The click that follows the drop must not select the item.
      dragged = true;
      setTimeout(() => (dragged = false));
      if (d && d.target !== null && d.target !== d.from) {
        const target = d.target;
        run(() => api.moveConnection(d.id, target));
        if (collapsed.includes(target)) toggle(target);
      }
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape" && menu) menu = null;
    if ((e.metaKey || e.ctrlKey) && e.key === "n") {
      e.preventDefault();
      newConnection(selected?.group ?? "");
    }
  }
</script>

<svelte:window
  {onkeydown}
  onmousedown={(e) => menu && !(e.target as Element).closest(".menu") && (menu = null)}
  onblur={() => (menu = null)}
/>

{#snippet item(c: ConnectionProfile, nested: boolean)}
  <button
    class="item"
    class:nested
    class:active={c.id === selectedId}
    class:dragging={drag?.id === c.id}
    onclick={() => !dragged && (selectedId = c.id)}
    ondblclick={() => app.connect(c.id)}
    onpointerdown={(e) => startDrag(e, c)}
    oncontextmenu={(e) => connectionMenu(e, c)}
  >
    <span class="swatch" style:background={c.color || "var(--muted)"}></span>
    <span class="text">
      <span class="name">{c.name || "Untitled"}</span>
      <span class="sub muted">{kindLabel[c.kind]} · {subtitle(c)}</span>
    </span>
    {#if app.connecting === c.id}<span class="muted">…</span>{/if}
  </button>
{/snippet}

<div class="screen">
  <aside class="list">
    <div class="titlebar" class:mac={isMac} data-tauri-drag-region>
      <span class="app-name" data-tauri-drag-region>Tablory</span>
    </div>
    <div
      class="items"
      class:drop-over={drag?.target === "" && drag.from !== ""}
      data-drop=""
      oncontextmenu={listMenu}
      role="presentation"
    >
      {#each sections[0].list as c (c.id)}
        {@render item(c, false)}
      {/each}

      {#each sections.slice(1) as { group: g, list } (g)}
        {@const open = !collapsed.includes(g)}
        <div class="group" class:drop-over={drag?.target === g && drag.from !== g} data-drop={g} role="group">
          <div class="group-head" oncontextmenu={(e) => groupMenu(e, g)} role="presentation">
            {#if renaming === g}
              <span class="chevron" class:closed={!open}>▾</span>
              <input
                class="field group-input"
                value={g}
                use:focus
                onblur={(e) => finishRename(g, e.currentTarget.value)}
                onkeydown={(e) => inputKeys(e, () => (renaming = null))}
              />
            {:else}
              <button class="group-name" aria-expanded={open} onclick={() => toggle(g)} ondblclick={() => (renaming = g)}>
                <span class="chevron" class:closed={!open}>▾</span>
                <span class="label">{g}</span>
              </button>
              <span class="count muted">{list.length}</span>
            {/if}
          </div>
          {#if open}
            {#each list as c (c.id)}
              {@render item(c, true)}
            {:else}
              <p class="hint muted">Drag connections here</p>
            {/each}
          {/if}
        </div>
      {/each}

      {#if creating}
        <div class="group">
          <div class="group-head">
            <span class="chevron">▾</span>
            <input
              class="field group-input"
              placeholder="Group name"
              use:focus
              onblur={(e) => finishNewGroup(e.currentTarget.value)}
              onkeydown={(e) => inputKeys(e, () => (creating = null))}
            />
          </div>
        </div>
      {/if}

      {#if app.connections.length === 0 && app.groups.length === 0 && !creating}
        <p class="empty muted">No saved connections yet. Fill in the form to add your first database.</p>
      {/if}
    </div>
    {#if loadError}<p class="error-text pad">{loadError}</p>{/if}
    {#if notice}
      <div class="notice pad" class:error-text={!notice.ok}>
        <span>{notice.text}</span>
        <button class="link muted" onclick={() => (notice = null)} title="Dismiss">✕</button>
        {#each notice.detail as d}<div class="muted small">{d}</div>{/each}
      </div>
    {/if}
    <div class="foot">
      <button class="btn" onclick={() => newConnection()} title="New connection (⌘N)">New connection</button>
      <button class="link muted" onclick={importNavicat} title="Import connections exported from Navicat (.ncx)">
        Import…
      </button>
      <span class="spacer"></span>
      <button class="link muted" onclick={() => invoke("check_updates")} title="Check for updates">
        {version ? `v${version}` : "Updates"}
      </button>
    </div>
  </aside>
  <main class="form-pane">
    <div class="titlebar" data-tauri-drag-region></div>
    {#key `${selectedId ?? "new"}-${formKey}`}
      <ConnectionForm
        profile={selected}
        group={newGroup}
        onsaved={(p) => (selectedId = p.id)}
        ondeleted={() => {
          selectedId = app.connections[0]?.id ?? null;
          formKey += 1;
        }}
      />
    {/key}
  </main>
</div>

{#if drag}
  <div class="ghost" style:left="{drag.x + 12}px" style:top="{drag.y + 8}px">
    {drag.name}
    {#if drag.target !== null && drag.target !== drag.from}
      <span class="muted">→ {drag.target || "No group"}</span>
    {/if}
  </div>
{/if}

{#if menu}
  <div class="menu" style:left="{menu.x}px" style:top="{menu.y}px" role="menu">
    {#each menu.items as m}
      {#if m === "sep"}
        <hr />
      {:else}
        <button
          role="menuitem"
          class:danger={m.danger}
          onclick={() => {
            menu = null;
            m.action();
          }}>{m.label}</button
        >
      {/if}
    {/each}
  </div>
{/if}

<style>
  .screen {
    display: grid;
    grid-template-columns: 260px 1fr;
    height: 100vh;
  }
  .list {
    display: flex;
    flex-direction: column;
    background: var(--sidebar);
    border-right: 1px solid var(--border);
    min-height: 0;
  }
  .titlebar {
    height: 38px;
    flex: none;
    display: flex;
    align-items: center;
    padding: 0 12px;
  }
  .titlebar.mac {
    padding-left: 80px;
  }
  .app-name {
    font-weight: 600;
  }
  .items {
    flex: 1;
    overflow: auto;
    padding: 4px 8px;
    user-select: none;
    outline: none;
  }
  .item {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 7px 8px;
    border: 0;
    border-radius: 6px;
    background: none;
    text-align: left;
  }
  .item:hover {
    background: var(--hover);
  }
  .item.nested {
    padding-left: 22px;
  }
  .item.active {
    background: var(--selection);
  }
  .item.dragging {
    opacity: 0.45;
  }
  .group {
    margin-top: 6px;
    border-radius: 6px;
  }
  .drop-over {
    box-shadow: inset 0 0 0 2px var(--accent);
    background: color-mix(in srgb, var(--accent) 8%, transparent);
  }
  .group-head {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 2px 6px 2px 4px;
    font-size: 11.5px;
    font-weight: 600;
    color: var(--muted);
  }
  .group-name {
    display: flex;
    align-items: center;
    gap: 4px;
    flex: 1;
    min-width: 0;
    padding: 2px 0;
    border: 0;
    background: none;
    color: inherit;
    font: inherit;
    text-align: left;
  }
  .label {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }
  .chevron {
    display: inline-block;
    width: 12px;
    flex: none;
    transition: transform 0.1s;
  }
  .chevron.closed {
    transform: rotate(-90deg);
  }
  .group-input {
    flex: 1;
    min-width: 0;
    height: 22px;
    font-size: 12px;
  }
  .count {
    font-weight: 400;
  }
  .hint {
    margin: 0;
    padding: 4px 8px 6px 22px;
    font-size: 11.5px;
  }
  .swatch {
    width: 10px;
    height: 10px;
    border-radius: 3px;
    flex: none;
  }
  .text {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .name,
  .sub {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .name {
    font-weight: 500;
  }
  .sub {
    font-size: 11.5px;
  }
  .empty {
    padding: 8px;
  }
  .pad {
    padding: 0 12px;
  }
  .foot {
    display: flex;
    align-items: center;
    padding: 10px 12px;
    border-top: 1px solid var(--border);
  }
  .notice {
    font-size: 12px;
    padding-bottom: 8px;
  }
  .notice .link {
    margin-left: 4px;
  }
  .small {
    font-size: 11px;
  }
  .foot .link {
    margin-left: 6px;
  }
  .spacer {
    flex: 1;
  }
  .link {
    border: 0;
    background: none;
    font-size: 12px;
    padding: 2px 4px;
    border-radius: 4px;
  }
  .link:hover {
    color: var(--text);
    background: var(--hover);
  }
  .form-pane {
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow: auto;
  }
  .ghost {
    position: fixed;
    z-index: 60;
    pointer-events: none;
    padding: 4px 10px;
    border-radius: 6px;
    background: var(--panel);
    border: 1px solid var(--border);
    box-shadow: 0 6px 20px rgb(0 0 0 / 0.18);
    font-size: 12px;
    white-space: nowrap;
  }
  .menu {
    position: fixed;
    z-index: 50;
    min-width: 200px;
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
    text-align: left;
  }
  .menu button:hover {
    background: var(--accent);
    color: var(--accent-text);
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
