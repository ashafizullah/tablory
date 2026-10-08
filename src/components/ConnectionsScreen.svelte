<script lang="ts">
  import { app } from "../lib/state/app.svelte";
  import { onMount } from "svelte";
  import { invoke, isMac } from "../lib/api";
  import { kindLabel } from "../lib/cells";
  import type { ConnectionProfile } from "../lib/types";
  import ConnectionForm from "./ConnectionForm.svelte";

  let { loadError }: { loadError: string | null } = $props();

  // null = new connection form.
  let selectedId = $state<string | null>(null);
  let formKey = $state(0);
  let version = $state("");

  onMount(() => {
    invoke<string>("app_version").then((v) => (version = v)).catch(() => {});
  });
  let selected = $derived(app.connections.find((c) => c.id === selectedId) ?? null);

  $effect(() => {
    if (selectedId === null && app.connections.length > 0 && formKey === 0) selectedId = app.connections[0].id;
  });

  function subtitle(c: ConnectionProfile) {
    if (c.kind === "sqlite") return c.file.split(/[\\/]/).pop() || "SQLite";
    const host = `${c.host || "localhost"}${c.port ? ":" + c.port : ""}`;
    return c.ssh.enabled ? `${host} via ${c.ssh.host}` : host;
  }

  function newConnection() {
    selectedId = null;
    formKey += 1;
  }

  function onkeydown(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key === "n") {
      e.preventDefault();
      newConnection();
    }
  }
</script>

<svelte:window {onkeydown} />

<div class="screen">
  <aside class="list">
    <div class="titlebar" class:mac={isMac} data-tauri-drag-region>
      <span class="app-name" data-tauri-drag-region>Tablory</span>
    </div>
    <div class="items">
      {#each app.connections as c (c.id)}
        <button
          class="item"
          class:active={c.id === selectedId}
          onclick={() => (selectedId = c.id)}
          ondblclick={() => app.connect(c.id)}
        >
          <span class="swatch" style:background={c.color || "var(--muted)"}></span>
          <span class="text">
            <span class="name">{c.name || "Untitled"}</span>
            <span class="sub muted">{kindLabel[c.kind]} · {subtitle(c)}</span>
          </span>
          {#if app.connecting === c.id}<span class="muted">…</span>{/if}
        </button>
      {:else}
        <p class="empty muted">No saved connections yet. Fill in the form to add your first database.</p>
      {/each}
    </div>
    {#if loadError}<p class="error-text pad">{loadError}</p>{/if}
    <div class="foot">
      <button class="btn" onclick={newConnection} title="New connection (⌘N)">New connection</button>
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
        onsaved={(p) => (selectedId = p.id)}
        ondeleted={() => {
          selectedId = app.connections[0]?.id ?? null;
          formKey += 1;
        }}
      />
    {/key}
  </main>
</div>

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
  .item.active {
    background: var(--selection);
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
</style>
