<script lang="ts">
  import type { TableStructure } from "../lib/types";

  let { structure }: { structure: TableStructure } = $props();
</script>

<div class="sv">
  <section>
    <h2>Columns <span class="muted">{structure.columns.length}</span></h2>
    <table>
      <thead>
        <tr><th>Name</th><th>Type</th><th>Nullable</th><th>Default</th><th>Key</th></tr>
      </thead>
      <tbody>
        {#each structure.columns as c}
          <tr>
            <td class="strong">{c.name}</td>
            <td class="mono">{c.data_type}</td>
            <td>{c.nullable ? "YES" : "NO"}</td>
            <td class="mono" class:muted={c.default === null}>{c.default ?? "—"}</td>
            <td>{c.primary_key ? "PRIMARY" : ""}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  </section>

  <section>
    <h2>Indexes <span class="muted">{structure.indexes.length}</span></h2>
    {#if structure.indexes.length}
      <table>
        <thead><tr><th>Name</th><th>Columns</th><th>Unique</th></tr></thead>
        <tbody>
          {#each structure.indexes as ix}
            <tr>
              <td class="strong">{ix.name}{ix.primary ? " (primary)" : ""}</td>
              <td class="mono">{ix.columns.join(", ")}</td>
              <td>{ix.unique ? "YES" : "NO"}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {:else}
      <p class="muted">No indexes.</p>
    {/if}
  </section>

  <section>
    <h2>Foreign keys <span class="muted">{structure.foreign_keys.length}</span></h2>
    {#if structure.foreign_keys.length}
      <table>
        <thead><tr><th>Name</th><th>Columns</th><th>References</th></tr></thead>
        <tbody>
          {#each structure.foreign_keys as fk}
            <tr>
              <td class="strong">{fk.name}</td>
              <td class="mono">{fk.columns.join(", ")}</td>
              <td class="mono">{fk.ref_table} ({fk.ref_columns.join(", ")})</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {:else}
      <p class="muted">No foreign keys.</p>
    {/if}
  </section>
</div>

<style>
  .sv {
    flex: 1;
    overflow: auto;
    padding: 12px 16px;
    -webkit-user-select: text;
    user-select: text;
  }
  section + section {
    margin-top: 20px;
  }
  h2 {
    font-size: 13px;
    margin: 0 0 8px;
  }
  table {
    border-collapse: collapse;
    width: 100%;
  }
  th,
  td {
    text-align: left;
    padding: 4px 10px;
    border-bottom: 1px solid var(--grid-line);
    white-space: nowrap;
  }
  th {
    font-weight: 600;
    color: var(--muted);
    font-size: 12px;
    border-bottom-color: var(--border);
  }
  .strong {
    font-weight: 500;
  }
  .mono {
    font-family: var(--mono);
    font-size: 12px;
  }
</style>
