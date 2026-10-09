<script lang="ts">
  import type { Safety } from "../lib/types";

  let { safety }: { safety: Exclude<Safety, "normal"> } = $props();
</script>

<span
  class="badge {safety}"
  title={safety === "production" ? "Production: changes ask for confirmation" : "Read-only: changes are blocked"}
  >{safety === "production" ? "PRODUCTION" : "READ-ONLY"}</span
>

<style>
  .badge {
    flex: none;
    padding: 1px 6px;
    border-radius: 4px;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.04em;
    line-height: 16px;
  }
  .production {
    background: var(--danger);
    color: #fff;
  }
  .readonly {
    border: 1px solid var(--border);
    color: var(--muted);
  }
</style>
