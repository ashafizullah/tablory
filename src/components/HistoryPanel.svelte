<script lang="ts">
  import { app } from "../lib/state/app.svelte";
  import { clearHistory, loadHistory, removeHistory, type HistoryEntry } from "../lib/history";
  import { preview } from "../lib/sqlguard";

  let {
    version,
    oninsert,
    onclose,
  }: {
    /** Bumped by the editor after each run, to reload the list. */
    version: number;
    oninsert: (sql: string) => void;
    onclose: () => void;
  } = $props();

  const connectionId = $derived(app.session!.connection_id);
  let entries = $state<HistoryEntry[]>([]);
  let search = $state("");
  let searchBox: HTMLInputElement | undefined = $state();

  $effect(() => {
    void version;
    entries = loadHistory(connectionId);
  });
  $effect(() => searchBox?.focus());

  const shown = $derived.by(() => {
    const q = search.trim().toLowerCase();
    return q ? entries.filter((e) => e.sql.toLowerCase().includes(q)) : entries;
  });

  function when(at: number) {
    const d = new Date(at);
    const today = new Date().toDateString() === d.toDateString();
    const time = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
    return today ? time : `${d.toLocaleDateString([], { day: "numeric", month: "short" })} ${time}`;
  }

  async function clear() {
    const ok = await app.ask("Clear the query history of this connection?", { ok: "Clear", danger: true });
    if (!ok) return;
    clearHistory(connectionId);
    entries = [];
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape") onclose();
  }
</script>

<aside class="history" aria-label="Query history">
  <header>
    <strong>History</strong>
    <span class="muted small">{entries.length.toLocaleString()}</span>
    <span class="grow"></span>
    <button class="link muted small" onclick={clear} disabled={entries.length === 0}>Clear</button>
    <button class="close" onclick={onclose} aria-label="Close history">×</button>
  </header>
  <input class="field search" placeholder="Search history" bind:value={search} bind:this={searchBox} {onkeydown} />
  <ul>
    {#each shown as e (e.at)}
      <li>
        <button class="entry" onclick={() => oninsert(e.sql)} title="Insert into the editor">
          <span class="meta">
            <span class:error-text={e.error}>{e.error ? "failed" : e.rows != null ? `${e.rows.toLocaleString()} rows` : "ok"}</span>
            <span class="muted">· {e.ms.toLocaleString()} ms · {e.database || "default"}</span>
            <span class="grow"></span>
            <span class="muted">{when(e.at)}</span>
          </span>
          <code>{preview(e.sql, 220)}</code>
        </button>
        <span class="actions">
          <button class="link small" onclick={() => app.newQuery(e.sql)} title="Open in a new tab">New tab</button>
          <button class="link small" onclick={() => navigator.clipboard.writeText(e.sql)}>Copy</button>
          <button class="link small muted" onclick={() => (entries = removeHistory(connectionId, e.at))}>Remove</button>
        </span>
      </li>
    {:else}
      <li class="empty muted">{entries.length ? "No matches." : "Queries you run show up here."}</li>
    {/each}
  </ul>
</aside>

<style>
  .history {
    position: absolute;
    top: 0;
    right: 0;
    bottom: 0;
    z-index: 20;
    width: min(380px, 90%);
    display: flex;
    flex-direction: column;
    background: var(--panel);
    border-left: 1px solid var(--border);
    box-shadow: -8px 0 24px rgb(0 0 0 / 0.12);
  }
  header {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 8px 6px 12px;
    border-bottom: 1px solid var(--border);
  }
  .grow {
    flex: 1;
  }
  .small {
    font-size: 12px;
  }
  .link {
    border: 0;
    background: none;
    padding: 0 4px;
    color: var(--accent);
    cursor: pointer;
  }
  .link.muted {
    color: var(--muted);
  }
  .close {
    border: 0;
    background: none;
    color: var(--muted);
    width: 22px;
    height: 22px;
    border-radius: 4px;
    font-size: 15px;
  }
  .close:hover {
    background: var(--hover);
  }
  .search {
    margin: 8px;
  }
  ul {
    flex: 1;
    overflow: auto;
    margin: 0;
    padding: 0 0 8px;
    list-style: none;
  }
  li {
    border-bottom: 1px solid var(--grid-line);
  }
  .entry {
    display: block;
    width: 100%;
    padding: 6px 12px 2px;
    border: 0;
    background: none;
    text-align: left;
    color: var(--text);
  }
  .entry:hover {
    background: var(--hover);
  }
  .meta {
    display: flex;
    gap: 4px;
    font-size: 11px;
    margin-bottom: 2px;
  }
  code {
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
    font-family: var(--mono);
    font-size: 12px;
    word-break: break-all;
  }
  .actions {
    display: flex;
    gap: 4px;
    padding: 0 8px 6px;
  }
  .empty {
    padding: 16px 12px;
    border: 0;
  }
</style>
