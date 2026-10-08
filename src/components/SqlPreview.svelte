<script lang="ts">
  let {
    sql,
    committing,
    oncommit,
    onclose,
  }: { sql: string; committing: boolean; oncommit: () => void; onclose: () => void } = $props();

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape") onclose();
  }
</script>

<svelte:window {onkeydown} />

<div class="backdrop">
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="preview-title">
    <h2 id="preview-title">Changes to commit</h2>
    <p class="muted">These statements run in one transaction. If any of them fails, nothing is saved.</p>
    <pre>{sql}</pre>
    <div class="actions">
      <button class="btn" onclick={() => navigator.clipboard.writeText(sql)}>Copy</button>
      <span class="grow"></span>
      <button class="btn" onclick={onclose}>Close</button>
      <button class="btn primary" onclick={oncommit} disabled={committing}>{committing ? "Saving…" : "Commit"}</button>
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgb(0 0 0 / 0.25);
    display: grid;
    place-items: center;
    z-index: 90;
  }
  .dialog {
    width: min(760px, 90vw);
    max-height: 80vh;
    display: flex;
    flex-direction: column;
    padding: 16px;
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 10px;
    box-shadow: 0 12px 40px rgb(0 0 0 / 0.25);
  }
  h2 {
    font-size: 15px;
    margin: 0 0 4px;
  }
  p {
    margin: 0 0 10px;
    font-size: 12px;
  }
  pre {
    flex: 1;
    overflow: auto;
    margin: 0;
    padding: 10px;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 6px;
    font-family: var(--mono);
    font-size: 12px;
    -webkit-user-select: text;
    user-select: text;
  }
  .actions {
    display: flex;
    gap: 8px;
    margin-top: 12px;
  }
  .grow {
    flex: 1;
  }
</style>
