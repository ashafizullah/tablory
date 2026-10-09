<script lang="ts">
  import { onMount } from "svelte";
  import { app } from "./lib/state/app.svelte";
  import { errorText } from "./lib/api";
  import ConnectionsScreen from "./components/ConnectionsScreen.svelte";
  import Workspace from "./components/Workspace.svelte";
  import ConfirmDialog from "./components/ConfirmDialog.svelte";

  let loadError = $state<string | null>(null);

  onMount(() => {
    app.loadConnections().catch((e) => (loadError = errorText(e)));
  });
</script>

{#if app.view === "workspace"}
  <Workspace />
{:else}
  <ConnectionsScreen {loadError} />
{/if}

{#if app.confirm}
  <ConfirmDialog />
{/if}
