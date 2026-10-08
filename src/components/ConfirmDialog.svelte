<script lang="ts">
  import { app } from "../lib/state/app.svelte";

  let okButton: HTMLButtonElement | undefined = $state();
  $effect(() => okButton?.focus());

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape") app.answer(false);
  }
</script>

<svelte:window {onkeydown} />

<div class="backdrop">
  <div class="dialog" role="alertdialog" aria-modal="true" aria-labelledby="confirm-msg">
    <p id="confirm-msg" class="msg">{app.confirm?.message}</p>
    {#if app.confirm?.detail}<p class="muted">{app.confirm.detail}</p>{/if}
    <div class="actions">
      <button class="btn" onclick={() => app.answer(false)}>Cancel</button>
      <button
        class="btn primary"
        class:danger-fill={app.confirm?.danger}
        bind:this={okButton}
        onclick={() => app.answer(true)}>{app.confirm?.ok}</button
      >
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
    z-index: 100;
  }
  .dialog {
    width: 360px;
    padding: 18px;
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 10px;
    box-shadow: 0 12px 40px rgb(0 0 0 / 0.25);
  }
  .msg {
    margin: 0 0 6px;
    font-weight: 600;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 16px;
  }
  .danger-fill {
    background: var(--danger);
    border-color: var(--danger);
  }
</style>
