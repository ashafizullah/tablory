<script lang="ts">
  import { onMount, tick } from "svelte";
  import { app } from "../lib/state/app.svelte";
  import { api, errorText } from "../lib/api";
  import type { RedisKey } from "../lib/types";
  import RedisKeyView from "./RedisKeyView.svelte";

  const session = $derived(app.session!);

  let pattern = $state("");
  let keys = $state<RedisKey[]>([]);
  let cursor = $state("0");
  let scanning = $state(false);
  let listError = $state<string | null>(null);
  let selected = $state<string | null>(null);
  let viewKey = $state(0);

  let creating = $state(false);
  let newKey = $state({ name: "", kind: "string", field: "", value: "" });
  let createError = $state<string | null>(null);

  let consoleOpen = $state(false);
  let line = $state("");
  let history = $state<{ cmd: string; out: string; error: boolean }[]>([]);
  let past: string[] = [];
  let pastIndex = -1;
  let consoleLog: HTMLDivElement | undefined = $state();
  let consoleInput: HTMLInputElement | undefined = $state();

  async function toggleConsole() {
    consoleOpen = !consoleOpen;
    await tick();
    if (consoleOpen) consoleInput?.focus();
  }

  const PAGE = 500;

  /** Scans until at least one page of matches or the end of the keyspace. */
  async function scan(reset: boolean) {
    if (scanning) return;
    scanning = true;
    listError = null;
    try {
      let next = reset ? "0" : cursor;
      const found: RedisKey[] = reset ? [] : [...keys];
      const target = found.length + PAGE;
      do {
        const page = await api.redisScan(session.id, next, pattern, 1000);
        found.push(...page.keys);
        next = page.cursor;
      } while (next !== "0" && found.length < target);
      keys = found;
      cursor = next;
    } catch (e) {
      listError = errorText(e);
    } finally {
      scanning = false;
    }
  }

  function open(key: string) {
    selected = key;
    viewKey += 1;
  }

  function onDeleted(key: string) {
    keys = keys.filter((k) => k.key !== key);
    selected = null;
  }

  function onRenamed(from: string, to: string) {
    keys = keys.map((k) => (k.key === from ? { ...k, key: to } : k));
    selected = to;
    viewKey += 1;
  }

  async function create() {
    createError = null;
    const { name, kind, field, value } = newKey;
    if (!name) {
      createError = "Enter a key name.";
      return;
    }
    const exists = await api.redis(session.id, "EXISTS", name);
    if (exists === 1) {
      createError = "A key with this name already exists.";
      return;
    }
    const args: Record<string, string[]> = {
      string: ["SET", name, value],
      hash: ["HSET", name, field || "field", value],
      list: ["RPUSH", name, value],
      set: ["SADD", name, value],
      zset: ["ZADD", name, field || "0", value],
    };
    try {
      await api.redis(session.id, ...args[kind]);
      keys = [...keys, { key: name, kind }].sort((a, b) => a.key.localeCompare(b.key));
      creating = false;
      newKey = { name: "", kind: "string", field: "", value: "" };
      open(name);
    } catch (e) {
      createError = errorText(e);
    }
  }

  async function runLine() {
    const cmd = line.trim();
    if (!cmd) return;
    past = [cmd, ...past.filter((p) => p !== cmd)].slice(0, 100);
    pastIndex = -1;
    line = "";
    try {
      const res = await api.redisLine(session.id, cmd);
      history.push({ cmd, out: typeof res === "string" ? res : JSON.stringify(res, null, 2), error: false });
    } catch (e) {
      history.push({ cmd, out: errorText(e), error: true });
    }
    await tick();
    consoleLog?.scrollTo({ top: consoleLog.scrollHeight });
    // Keep the open key in sync with what the command may have changed.
    if (selected) viewKey += 1;
  }

  function consoleKey(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      runLine();
    } else if (e.key === "ArrowUp" && past.length) {
      e.preventDefault();
      pastIndex = Math.min(past.length - 1, pastIndex + 1);
      line = past[pastIndex];
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      pastIndex = Math.max(-1, pastIndex - 1);
      line = pastIndex < 0 ? "" : past[pastIndex];
    }
  }

  onMount(() => scan(true));
</script>

<div class="rw">
  <aside class="keys">
    <div class="top">
      <div class="row">
        <input
          class="field grow mono"
          bind:value={pattern}
          placeholder="Pattern, e.g. user:*"
          aria-label="Key pattern"
          onkeydown={(e) => e.key === "Enter" && scan(true)}
        />
        <button class="btn ghost refresh" aria-label="Rescan keys" title="Rescan" onclick={() => scan(true)}>↻</button>
      </div>
      <button class="btn" onclick={() => (creating = !creating)}>+ Key</button>
    </div>

    {#if creating}
      <form class="create" onsubmit={(e) => (e.preventDefault(), create())}>
        <input class="field mono" bind:value={newKey.name} placeholder="Key name" aria-label="Key name" />
        <select class="field" bind:value={newKey.kind} aria-label="Type">
          <option value="string">String</option>
          <option value="hash">Hash</option>
          <option value="list">List</option>
          <option value="set">Set</option>
          <option value="zset">Sorted set</option>
        </select>
        {#if newKey.kind === "hash" || newKey.kind === "zset"}
          <input
            class="field mono"
            bind:value={newKey.field}
            placeholder={newKey.kind === "hash" ? "Field" : "Score"}
            aria-label={newKey.kind === "hash" ? "Field" : "Score"}
          />
        {/if}
        <input
          class="field mono"
          bind:value={newKey.value}
          placeholder={newKey.kind === "zset" || newKey.kind === "set" ? "Member" : "Value"}
          aria-label="Value"
        />
        {#if createError}<p class="error-text small">{createError}</p>{/if}
        <div class="row">
          <button type="button" class="btn" onclick={() => (creating = false)}>Cancel</button>
          <button type="submit" class="btn primary">Create</button>
        </div>
      </form>
    {/if}

    <div class="list">
      {#if listError}
        <p class="error-text pad">{listError}</p>
      {:else if keys.length === 0 && !scanning}
        <p class="muted pad">{pattern ? `No keys match “${pattern}”.` : `db ${session.database} is empty. Add a key with + Key.`}</p>
      {/if}
      {#each keys as k (k.key)}
        <button class="item" class:active={k.key === selected} onclick={() => open(k.key)} title={k.key}>
          <span class="tag t-{k.kind}">{k.kind}</span>
          <span class="name mono">{k.key}</span>
        </button>
      {/each}
      {#if scanning}
        <p class="muted pad">Scanning…</p>
      {:else if cursor !== "0"}
        <button class="btn more" onclick={() => scan(false)}>Load more</button>
      {/if}
    </div>
    <div class="foot muted">{keys.length.toLocaleString()} key{keys.length === 1 ? "" : "s"}{cursor !== "0" ? "+" : ""}</div>
  </aside>

  <main class="main">
    <div class="view">
      {#if selected}
        {#key `${selected}-${viewKey}`}
          <RedisKeyView key={selected} ondeleted={onDeleted} onrenamed={onRenamed} />
        {/key}
      {:else}
        <div class="placeholder muted">Select a key to see its value, or open the console to run commands.</div>
      {/if}
    </div>

    <section class="console" class:open={consoleOpen}>
      <button class="console-head" onclick={toggleConsole} aria-expanded={consoleOpen}>
        <span>{consoleOpen ? "▾" : "▸"} Console</span>
      </button>
      {#if consoleOpen}
        <div class="log mono" bind:this={consoleLog}>
          {#each history as h}
            <div class="cmd">&gt; {h.cmd}</div>
            <pre class:error-text={h.error}>{h.out}</pre>
          {:else}
            <p class="muted">Type a command, e.g. <span class="mono">HGETALL user:1</span>. ↑ recalls earlier ones.</p>
          {/each}
        </div>
        <input
          class="field mono line"
          bind:value={line}
          bind:this={consoleInput}
          onkeydown={consoleKey}
          placeholder="Command"
          aria-label="Redis command"
          autocapitalize="off"
          spellcheck="false"
        />
      {/if}
    </section>
  </main>
</div>

<style>
  .rw {
    flex: 1;
    display: grid;
    grid-template-columns: 280px 1fr;
    min-height: 0;
  }
  .keys {
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--sidebar);
    border-right: 1px solid var(--border);
  }
  .top,
  .create {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px;
  }
  .create {
    border-bottom: 1px solid var(--border);
  }
  .row {
    display: flex;
    gap: 6px;
  }
  .row .btn {
    flex: 1;
  }
  .grow {
    flex: 1;
  }
  .refresh {
    flex: none !important;
    padding: 0 6px;
    color: var(--muted);
  }
  .mono {
    font-family: var(--mono);
    font-size: 12px;
  }
  .small {
    font-size: 12px;
    margin: 0;
  }
  .list {
    flex: 1;
    overflow: auto;
    padding: 0 6px 8px;
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
  .tag {
    flex: none;
    width: 44px;
    font-size: 10px;
    text-align: center;
    border-radius: 3px;
    padding: 1px 0;
    color: var(--muted);
    border: 1px solid var(--border);
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
  .more {
    width: 100%;
    margin-top: 6px;
  }
  .foot {
    padding: 6px 10px;
    font-size: 12px;
    border-top: 1px solid var(--border);
  }
  .main {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    background: var(--panel);
  }
  .view {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .placeholder {
    flex: 1;
    display: grid;
    place-items: center;
    padding: 16px;
    text-align: center;
  }
  .console {
    flex: none;
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--border);
    background: var(--bg);
  }
  .console.open {
    height: 40%;
    min-height: 160px;
  }
  .console-head {
    border: 0;
    background: none;
    text-align: left;
    padding: 6px 10px;
    font-weight: 600;
    font-size: 12px;
  }
  .log {
    flex: 1;
    overflow: auto;
    padding: 0 10px;
    -webkit-user-select: text;
    user-select: text;
  }
  .cmd {
    color: var(--muted);
    margin-top: 6px;
  }
  .log pre {
    margin: 2px 0 0;
    white-space: pre-wrap;
    word-break: break-all;
  }
  .line {
    margin: 6px 8px 8px;
  }
</style>
