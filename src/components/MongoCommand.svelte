<script lang="ts">
  import { app, type Tab } from "../lib/state/app.svelte";
  import { api, errorText } from "../lib/api";
  import JsonEditor from "./JsonEditor.svelte";

  let { tab, active }: { tab: Extract<Tab, { kind: "mongo-command" }>; active: boolean } = $props();

  let running = $state(false);
  let output = $state<string | null>(null);
  let error = $state<string | null>(null);
  let duration = $state(0);
  let editor: JsonEditor | undefined = $state();

  const session = $derived(app.session!);

  async function run() {
    if (running || !app.schema) return;
    running = true;
    error = null;
    const start = performance.now();
    try {
      const res = await api.mongoCommand(session.id, app.schema, tab.text);
      output = JSON.stringify(res, null, 2);
      // Commands such as create/drop change the collection list.
      if (/"?(create|drop|renameCollection)"?\s*:/.test(tab.text)) app.loadTables();
    } catch (e) {
      output = null;
      error = errorText(e);
    } finally {
      duration = Math.round(performance.now() - start);
      running = false;
    }
  }

  $effect(() => {
    if (active) queueMicrotask(() => editor?.focus());
  });
</script>

<div class="mc">
  <div class="toolbar">
    <button class="btn primary" onclick={run} disabled={running || !app.schema}>
      Run <span class="kbd on-accent">⌘↵</span>
    </button>
    <span class="muted small">runCommand on <strong>{app.schema || "no database"}</strong></span>
    <span class="grow"></span>
    <span class="muted small">JSON; unquoted keys are fine, e.g. {"{ count: \"orders\", query: { paid: true } }"}</span>
  </div>
  <div class="input">
    <JsonEditor bind:this={editor} value={tab.text} onchange={(v) => (tab.text = v)} onsubmit={run} />
  </div>
  <div class="output">
    {#if running}
      <div class="placeholder muted">Running…</div>
    {:else if error}
      <p class="error-text pad">{error}</p>
    {:else if output !== null}
      {#key output}<JsonEditor value={output} readonly />{/key}
    {:else}
      <div class="placeholder muted">Press ⌘↵ to run the command against the database selected in the sidebar.</div>
    {/if}
  </div>
  <footer class="bar muted">
    {#if output !== null || error}<span>{duration} ms</span>{/if}
  </footer>
</div>

<style>
  .mc {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    border-bottom: 1px solid var(--border);
    background: var(--bg);
  }
  .small {
    font-size: 12px;
  }
  .grow {
    flex: 1;
  }
  .on-accent {
    color: inherit;
    opacity: 0.75;
  }
  .input {
    height: 200px;
    display: flex;
    border-bottom: 1px solid var(--border);
  }
  .output {
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
  .pad {
    padding: 12px 14px;
    margin: 0;
  }
  .bar {
    flex: none;
    display: flex;
    align-items: center;
    height: 26px;
    padding: 0 10px;
    font-size: 12px;
    border-top: 1px solid var(--border);
    background: var(--bg);
  }
</style>
