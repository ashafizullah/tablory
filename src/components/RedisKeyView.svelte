<script lang="ts">
  import { onMount } from "svelte";
  import { app } from "../lib/state/app.svelte";
  import { api, errorText } from "../lib/api";
  import type { RedisValue } from "../lib/types";

  let {
    key,
    ondeleted,
    onrenamed,
  }: { key: string; ondeleted: (key: string) => void; onrenamed: (from: string, to: string) => void } = $props();

  const session = $derived(app.session!);
  /** Placeholder used to delete a list item by index (LSET + LREM). */
  const LIST_TOMBSTONE = "__tablory_deleted__";

  let data = $state<RedisValue | null>(null);
  let error = $state<string | null>(null);
  let actionError = $state<string | null>(null);
  let busy = $state(false);

  let stringText = $state("");
  let ttlText = $state("");
  let renaming = $state<string | null>(null);
  let edit = $state<{ row: number; col: number; text: string } | null>(null);
  let add = $state({ a: "", b: "" });

  type Row = { cells: string[]; binary: boolean };

  const isBin = (v: unknown) => typeof v === "object" && v !== null && "$bin" in v;
  const show = (v: unknown) =>
    isBin(v) ? `(binary ${(v as { $bin: number }).$bin} bytes)` : typeof v === "string" ? v : JSON.stringify(v);

  const columns = $derived(
    ({
      hash: ["Field", "Value"],
      list: ["Index", "Value"],
      set: ["Member"],
      zset: ["Member", "Score"],
      stream: ["ID", "Fields"],
    } as Record<string, string[]>)[data?.kind ?? ""] ?? [],
  );

  const rows = $derived.by((): Row[] => {
    if (!data || !Array.isArray(data.value)) return [];
    const v = data.value as unknown[];
    switch (data.kind) {
      case "hash":
      case "zset":
      case "stream":
        return v.map((p) => {
          const [a, b] = p as [unknown, unknown];
          return { cells: [show(a), show(b)], binary: isBin(a) || isBin(b) };
        });
      case "list":
        return v.map((x, i) => ({ cells: [String(i), show(x)], binary: isBin(x) }));
      case "set":
        return v.map((x) => ({ cells: [show(x)], binary: isBin(x) }));
    }
    return [];
  });

  /** Which column of a row can be edited in place. */
  function editableCol(c: number) {
    if (!data) return false;
    return (
      (data.kind === "hash" && c === 1) ||
      (data.kind === "list" && c === 1) ||
      (data.kind === "set" && c === 0) ||
      (data.kind === "zset" && (c === 0 || c === 1))
    );
  }

  async function load() {
    error = null;
    try {
      data = await api.redisGet(session.id, key);
      stringText = typeof data.value === "string" ? data.value : "";
      ttlText = data.ttl > 0 ? String(data.ttl) : "";
    } catch (e) {
      error = errorText(e);
    }
  }

  /** Runs commands in order, then reloads. */
  async function run(...cmds: string[][]) {
    busy = true;
    actionError = null;
    try {
      for (const c of cmds) await api.redis(session.id, ...c);
      edit = null;
      await load();
    } catch (e) {
      actionError = errorText(e);
    } finally {
      busy = false;
    }
  }

  function saveString() {
    // KEEPTTL keeps an existing expiry (Redis 6+).
    run(["SET", key, stringText, "KEEPTTL"]);
  }

  function saveTtl() {
    const n = Number(ttlText);
    if (ttlText.trim() === "") run(["PERSIST", key]);
    else if (Number.isInteger(n) && n > 0) run(["EXPIRE", key, String(n)]);
    else actionError = "TTL must be a whole number of seconds, or empty for no expiry.";
  }

  async function rename() {
    const to = renaming?.trim();
    renaming = null;
    if (!to || to === key) return;
    actionError = null;
    try {
      const ok = await api.redis(session.id, "RENAMENX", key, to);
      if (ok === 0) actionError = `A key named “${to}” already exists.`;
      else onrenamed(key, to);
    } catch (e) {
      actionError = errorText(e);
    }
  }

  async function removeKey() {
    const ok = await app.ask(`Delete key “${key}”?`, { detail: "This cannot be undone.", ok: "Delete", danger: true });
    if (!ok) return;
    try {
      await api.redis(session.id, "DEL", key);
      ondeleted(key);
    } catch (e) {
      actionError = errorText(e);
    }
  }

  function commitEdit() {
    if (!edit || !data) return;
    const { row, col, text } = edit;
    const [a, b] = rows[row].cells;
    if (text === rows[row].cells[col]) {
      edit = null;
      return;
    }
    switch (data.kind) {
      case "hash":
        return run(["HSET", key, a, text]);
      case "list":
        return run(["LSET", key, a, text]);
      case "set":
        return run(["SREM", key, a], ["SADD", key, text]);
      case "zset":
        return col === 1 ? run(["ZADD", key, text, a]) : run(["ZREM", key, a], ["ZADD", key, b, text]);
    }
  }

  function removeRow(i: number) {
    if (!data) return;
    const [a] = rows[i].cells;
    switch (data.kind) {
      case "hash":
        return run(["HDEL", key, a]);
      case "list":
        return run(["LSET", key, a, LIST_TOMBSTONE], ["LREM", key, "1", LIST_TOMBSTONE]);
      case "set":
        return run(["SREM", key, a]);
      case "zset":
        return run(["ZREM", key, a]);
      case "stream":
        return run(["XDEL", key, a]);
    }
  }

  function addRow(e: SubmitEvent) {
    e.preventDefault();
    if (!data) return;
    const { a, b } = add;
    const cmd: Record<string, string[]> = {
      hash: ["HSET", key, a, b],
      list: ["RPUSH", key, a],
      set: ["SADD", key, a],
      zset: ["ZADD", key, b || "0", a],
      stream: ["XADD", key, "*", a, b],
    };
    run(cmd[data.kind]).then(() => {
      if (!actionError) add = { a: "", b: "" };
    });
  }

  const addLabels = $derived(
    ({
      hash: ["Field", "Value"],
      list: ["Value"],
      set: ["Member"],
      zset: ["Member", "Score"],
      stream: ["Field", "Value"],
    } as Record<string, string[]>)[data?.kind ?? ""] ?? [],
  );

  function ttlLabel(ttl: number) {
    if (ttl < 0) return "no expiry";
    if (ttl < 120) return `${ttl}s`;
    if (ttl < 7200) return `${Math.round(ttl / 60)}m`;
    return `${Math.round(ttl / 3600)}h`;
  }

  onMount(load);
</script>

<div class="kv">
  {#if error}
    <div class="placeholder"><p class="error-text">{error}</p></div>
  {:else if !data}
    <div class="placeholder muted">Loading…</div>
  {:else}
    <header class="head">
      <span class="kind">{data.kind}</span>
      {#if renaming !== null}
        <input
          class="field mono grow"
          bind:value={renaming}
          aria-label="New key name"
          onkeydown={(e) => {
            if (e.key === "Enter") rename();
            if (e.key === "Escape") renaming = null;
          }}
        />
        <button class="btn" onclick={rename}>Rename</button>
      {:else}
        <strong class="mono keyname" title={key}>{key}</strong>
        <button class="btn ghost" onclick={() => (renaming = key)}>Rename</button>
      {/if}
      <span class="grow"></span>
      <button class="btn ghost" aria-label="Reload key" title="Reload" onclick={load}>↻</button>
      <button class="btn danger" onclick={removeKey}>Delete</button>
    </header>

    <div class="meta">
      <span class="muted">{data.kind === "string" ? `${data.length.toLocaleString()} bytes` : `${data.length.toLocaleString()} items`}</span>
      <span class="muted">TTL {ttlLabel(data.ttl)}</span>
      <label class="ttl">
        <span class="muted">Expire in</span>
        <input
          class="field mono"
          bind:value={ttlText}
          placeholder="seconds"
          aria-label="TTL in seconds"
          onkeydown={(e) => e.key === "Enter" && saveTtl()}
        />
      </label>
      <button class="btn" onclick={saveTtl} disabled={busy}>Set TTL</button>
    </div>

    {#if actionError}<div class="banner error-text">{actionError}</div>{/if}
    {#if data.truncated}
      <div class="banner muted">Showing the first {rows.length.toLocaleString()} of {data.length.toLocaleString()} items.</div>
    {/if}

    {#if data.kind === "string"}
      {#if isBin(data.value)}
        <div class="placeholder muted">{show(data.value)} — binary values can be changed from the console.</div>
      {:else}
        <textarea class="text mono" bind:value={stringText} aria-label="Value" spellcheck="false"></textarea>
        <div class="actions">
          <span class="grow"></span>
          <button class="btn" onclick={() => (stringText = String(data?.value ?? ""))} disabled={stringText === data.value}>Revert</button>
          <button class="btn primary" onclick={saveString} disabled={busy || stringText === data.value}>Save</button>
        </div>
      {/if}
    {:else if columns.length}
      <div class="table-wrap">
        <table>
          <thead>
            <tr>
              {#each columns as c}<th>{c}</th>{/each}
              <th class="act" aria-label="Actions"></th>
            </tr>
          </thead>
          <tbody>
            {#each rows as r, i}
              <tr>
                {#each r.cells as cell, c}
                  <td
                    class="mono"
                    class:editable={editableCol(c) && !r.binary}
                    ondblclick={() => editableCol(c) && !r.binary && (edit = { row: i, col: c, text: cell })}
                  >
                    {#if edit?.row === i && edit?.col === c}
                      <!-- svelte-ignore a11y_autofocus -->
                      <input
                        class="field mono cell-input"
                        bind:value={edit.text}
                        autofocus
                        aria-label="Edit {columns[c]}"
                        onkeydown={(e) => {
                          if (e.key === "Enter") commitEdit();
                          if (e.key === "Escape") edit = null;
                        }}
                        onblur={() => (edit = null)}
                      />
                    {:else}
                      {cell}
                    {/if}
                  </td>
                {/each}
                <td class="act">
                  <button class="btn ghost del" aria-label="Remove item" title="Remove" onclick={() => removeRow(i)} disabled={busy}
                    >×</button
                  >
                </td>
              </tr>
            {:else}
              <tr><td class="muted" colspan={columns.length + 1}>Empty.</td></tr>
            {/each}
          </tbody>
        </table>
      </div>
      <form class="add" onsubmit={addRow}>
        {#each addLabels as label, i}
          {#if i === 0}
            <input class="field mono grow" bind:value={add.a} placeholder={label} aria-label={label} />
          {:else}
            <input class="field mono grow" bind:value={add.b} placeholder={label} aria-label={label} />
          {/if}
        {/each}
        <button class="btn" type="submit" disabled={busy || !add.a}>Add</button>
      </form>
      <p class="hint muted">Double-click a value to edit it; Enter saves.</p>
    {:else}
      <div class="placeholder muted">Type “{data.kind}” is not shown here yet; use the console.</div>
    {/if}
  {/if}
</div>

<style>
  .kv {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .head,
  .meta,
  .actions,
  .add {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
  }
  .head {
    border-bottom: 1px solid var(--border);
  }
  .meta {
    font-size: 12px;
    border-bottom: 1px solid var(--border);
    background: var(--bg);
  }
  .kind {
    font-size: 11px;
    padding: 1px 6px;
    border: 1px solid var(--border);
    border-radius: 4px;
    color: var(--muted);
  }
  .keyname {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }
  .grow {
    flex: 1;
  }
  .mono {
    font-family: var(--mono);
    font-size: 12px;
  }
  .ttl {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-left: auto;
  }
  .ttl input {
    width: 100px;
  }
  .banner {
    padding: 6px 10px;
    font-size: 12px;
    border-bottom: 1px solid var(--border);
  }
  .placeholder {
    flex: 1;
    display: grid;
    place-items: center;
    padding: 16px;
    text-align: center;
  }
  .text {
    flex: 1;
    margin: 10px 10px 0;
    padding: 8px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--panel);
    resize: none;
    -webkit-user-select: text;
    user-select: text;
  }
  .table-wrap {
    flex: 1;
    overflow: auto;
    min-height: 0;
  }
  table {
    width: 100%;
    border-collapse: collapse;
  }
  th,
  td {
    text-align: left;
    padding: 0 10px;
    height: 26px;
    border-bottom: 1px solid var(--grid-line);
    max-width: 480px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  th {
    position: sticky;
    top: 0;
    background: var(--bg);
    font-size: 12px;
    color: var(--muted);
    font-weight: 600;
    border-bottom-color: var(--border);
  }
  td.editable {
    cursor: text;
  }
  .act {
    width: 36px;
    text-align: center;
  }
  .del {
    padding: 0 6px;
    height: 20px;
    color: var(--muted);
  }
  .cell-input {
    width: 100%;
    height: 22px;
  }
  .add {
    border-top: 1px solid var(--border);
    background: var(--bg);
  }
  .hint {
    font-size: 11.5px;
    margin: 0;
    padding: 0 10px 8px;
    background: var(--bg);
  }
</style>
