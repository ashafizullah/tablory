<script lang="ts">
  import { app } from "../lib/state/app.svelte";
  import { api, errorText } from "../lib/api";
  import { kindLabel } from "../lib/cells";
  import type { ConnectionProfile, DbKind, Secrets } from "../lib/types";

  let {
    profile,
    group = "",
    onsaved,
    ondeleted,
  }: {
    profile: ConnectionProfile | null;
    /** Group of a new connection. */
    group?: string;
    onsaved: (p: ConnectionProfile) => void;
    ondeleted: () => void;
  } = $props();

  const COLORS = ["#6e6e73", "#2563eb", "#16a34a", "#d97706", "#dc2626", "#9333ea"];
  const KINDS: DbKind[] = ["postgres", "mysql", "mssql", "sqlite", "redis", "mongodb"];
  const DEFAULT_PORT: Partial<Record<DbKind, number>> = {
    postgres: 5432,
    mysql: 3306,
    mssql: 1433,
    redis: 6379,
    mongodb: 27017,
  };
  const USER_HINT: Partial<Record<DbKind, string>> = {
    postgres: "postgres",
    mysql: "root",
    mssql: "sa",
    redis: "default (optional)",
    mongodb: "optional",
  };
  const DB_HINT: Partial<Record<DbKind, string>> = {
    postgres: "postgres",
    mssql: "login's default (master)",
    redis: "0",
    mongodb: "optional",
  };

  function blank(): ConnectionProfile {
    return {
      id: "",
      name: "",
      group,
      kind: "postgres",
      color: COLORS[0],
      host: "127.0.0.1",
      port: 0,
      user: "",
      database: "",
      file: "",
      ssl_mode: "prefer",
      ssh: { enabled: false, host: "", port: 22, user: "", auth: "password", key_path: "" },
      uri: "",
      auth_source: "",
      windows_auth: false,
    };
  }

  // svelte-ignore state_referenced_locally
  let p = $state<ConnectionProfile>(profile ? structuredClone($state.snapshot(profile)) : blank());
  const saved = p.id !== "";
  const usesUri = $derived(p.kind === "mongodb" && p.uri.trim() !== "");

  // Saved secrets stay in the keychain; an untouched field means "keep".
  let password = $state("");
  let sshPassword = $state("");
  let sshPassphrase = $state("");
  let touched = $state({ password: false, ssh_password: false, ssh_passphrase: false });

  let busy = $state<"" | "test" | "save" | "connect">("");
  let status = $state<{ ok: boolean; text: string } | null>(null);

  function secrets(): Secrets {
    return {
      password: touched.password || !saved ? password : null,
      ssh_password: touched.ssh_password || !saved ? sshPassword : null,
      ssh_passphrase: touched.ssh_passphrase || !saved ? sshPassphrase : null,
    };
  }

  function defaultName() {
    if (p.kind === "sqlite") return p.file.split(/[\\/]/).pop() || "SQLite";
    return [p.database, p.host].filter(Boolean).join(" @ ") || kindLabel[p.kind];
  }

  async function test() {
    busy = "test";
    status = null;
    try {
      const db = await api.testConnection($state.snapshot(p), secrets());
      status = { ok: true, text: `Connected${db ? ` to ${db}` : ""}.` };
    } catch (e) {
      status = { ok: false, text: errorText(e) };
    } finally {
      busy = "";
    }
  }

  async function save(): Promise<ConnectionProfile | null> {
    if (!p.name.trim()) p.name = defaultName();
    try {
      const out = await api.saveConnection($state.snapshot(p), secrets());
      await app.loadConnections();
      onsaved(out);
      return out;
    } catch (e) {
      status = { ok: false, text: errorText(e) };
      return null;
    }
  }

  async function saveOnly() {
    busy = "save";
    status = null;
    if (await save()) status = { ok: true, text: "Saved." };
    busy = "";
  }

  async function saveAndConnect() {
    busy = "connect";
    status = null;
    const out = await save();
    if (out) {
      await app.connect(out.id);
      if (app.connectError) status = { ok: false, text: app.connectError };
    }
    busy = "";
  }

  async function remove() {
    const ok = await app.ask(`Delete “${p.name}”?`, {
      detail: "The saved password is removed from the keychain too.",
      ok: "Delete",
      danger: true,
    });
    if (!ok) return;
    await api.deleteConnection(p.id);
    await app.loadConnections();
    ondeleted();
  }

  async function browse(target: "file" | "key", create = false) {
    const path = await api.pickFile(create);
    if (!path) return;
    if (target === "file") p.file = path;
    else p.ssh.key_path = path;
  }

  function onsubmit(e: SubmitEvent) {
    e.preventDefault();
    saveAndConnect();
  }
</script>

<form class="form" {onsubmit}>
  <h1>{saved ? p.name || "Connection" : "New connection"}</h1>

  <div class="kinds" role="radiogroup" aria-label="Database type">
    {#each KINDS as k}
      <button
        type="button"
        role="radio"
        aria-checked={p.kind === k}
        class="kind"
        class:on={p.kind === k}
        onclick={() => (p.kind = k)}>{kindLabel[k]}</button
      >
    {/each}
  </div>

  <div class="grid">
    <label for="f-name">Name</label>
    <div class="row">
      <input id="f-name" class="field grow" bind:value={p.name} placeholder={defaultName()} />
      <div class="colors" role="radiogroup" aria-label="Color tag">
        {#each COLORS as c}
          <button
            type="button"
            role="radio"
            aria-checked={p.color === c}
            aria-label="Color {c}"
            class="color"
            class:on={p.color === c}
            style:background={c}
            onclick={() => (p.color = c)}
          ></button>
        {/each}
      </div>
    </div>

    {#if p.kind === "sqlite"}
      <label for="f-file">File</label>
      <div class="row">
        <input id="f-file" class="field grow" bind:value={p.file} placeholder="/path/to/database.sqlite" />
        <button type="button" class="btn" onclick={() => browse("file")}>Open…</button>
        <button type="button" class="btn" onclick={() => browse("file", true)}>New…</button>
      </div>
    {:else}
      {#if p.kind === "mongodb"}
        <label for="f-uri">URI</label>
        <input
          id="f-uri"
          class="field"
          bind:value={p.uri}
          autocapitalize="off"
          spellcheck="false"
          placeholder="mongodb+srv://user:pass@cluster.example.net/app (optional)"
        />
      {/if}
      {#if !usesUri}
      <label for="f-host">Host</label>
      <div class="row">
        <input id="f-host" class="field grow" bind:value={p.host} placeholder={p.kind === "mssql" ? "localhost or localhost\\SQLEXPRESS" : "127.0.0.1"} autocapitalize="off" />
        <label for="f-port" class="inline">Port</label>
        <input
          id="f-port"
          class="field port"
          type="number"
          min="0"
          max="65535"
          value={p.port || ""}
          placeholder={String(DEFAULT_PORT[p.kind] ?? "")}
          oninput={(e) => (p.port = Number(e.currentTarget.value) || 0)}
        />
      </div>

      {#if p.kind === "mssql"}
        <span class="label">Auth</span>
        <div class="row">
          <label class="check"><input type="radio" bind:group={p.windows_auth} value={false} /> SQL Server</label>
          <label class="check"><input type="radio" bind:group={p.windows_auth} value={true} /> Windows</label>
        </div>
      {/if}

      {#if !(p.kind === "mssql" && p.windows_auth)}
      <label for="f-user">User</label>
      <input id="f-user" class="field" bind:value={p.user} autocapitalize="off" placeholder={USER_HINT[p.kind] ?? ""} />

      <label for="f-pass">Password</label>
      <input
        id="f-pass"
        class="field"
        type="password"
        bind:value={password}
        oninput={() => (touched.password = true)}
        placeholder={saved ? "Saved in keychain (unchanged)" : ""}
      />
      {/if}

      {/if}

      <label for="f-db">{p.kind === "redis" ? "DB index" : "Database"}</label>
      <input
        id="f-db"
        class="field"
        class:narrow={p.kind === "redis"}
        bind:value={p.database}
        autocapitalize="off"
        placeholder={DB_HINT[p.kind] ?? "optional"}
      />

      {#if p.kind === "mongodb" && !usesUri}
        <label for="f-auth">Auth DB</label>
        <input id="f-auth" class="field narrow" bind:value={p.auth_source} autocapitalize="off" placeholder="admin" />
      {/if}

      {#if !usesUri}
        <label for="f-ssl">{p.kind === "redis" || p.kind === "mongodb" ? "TLS" : "SSL"}</label>
        <select id="f-ssl" class="field narrow" bind:value={p.ssl_mode}>
          {#if p.kind === "redis" || p.kind === "mongodb"}
            <option value="disable">Off</option>
            <option value="require">On</option>
          {:else}
            <option value="disable">Disable</option>
            <option value="prefer">Prefer</option>
            <option value="require">Require</option>
          {/if}
        </select>
      {/if}
    {/if}
  </div>

  {#if usesUri}
    <p class="muted note">The URI sets the hosts, user, password and TLS; SSH is not used with a URI.</p>
  {:else if p.kind !== "sqlite"}
    <fieldset class="ssh">
      <legend>
        <label class="check"><input type="checkbox" bind:checked={p.ssh.enabled} /> Connect over SSH</label>
      </legend>
      {#if p.ssh.enabled}
        <div class="grid">
          <label for="s-host">SSH host</label>
          <div class="row">
            <input id="s-host" class="field grow" bind:value={p.ssh.host} placeholder="bastion.example.com" autocapitalize="off" />
            <label for="s-port" class="inline">Port</label>
            <input
              id="s-port"
              class="field port"
              type="number"
              value={p.ssh.port}
              oninput={(e) => (p.ssh.port = Number(e.currentTarget.value) || 22)}
            />
          </div>
          <label for="s-user">SSH user</label>
          <input id="s-user" class="field" bind:value={p.ssh.user} autocapitalize="off" />

          <span class="label">Auth</span>
          <div class="row">
            <label class="check"><input type="radio" bind:group={p.ssh.auth} value="password" /> Password</label>
            <label class="check"><input type="radio" bind:group={p.ssh.auth} value="key" /> Private key</label>
          </div>

          {#if p.ssh.auth === "password"}
            <label for="s-pass">SSH password</label>
            <input
              id="s-pass"
              class="field"
              type="password"
              bind:value={sshPassword}
              oninput={() => (touched.ssh_password = true)}
              placeholder={saved ? "Saved in keychain (unchanged)" : ""}
            />
          {:else}
            <label for="s-key">Key file</label>
            <div class="row">
              <input id="s-key" class="field grow" bind:value={p.ssh.key_path} placeholder="~/.ssh/id_ed25519 (default)" />
              <button type="button" class="btn" onclick={() => browse("key")}>Choose…</button>
            </div>
            <label for="s-phrase">Passphrase</label>
            <input
              id="s-phrase"
              class="field"
              type="password"
              bind:value={sshPassphrase}
              oninput={() => (touched.ssh_passphrase = true)}
              placeholder={saved ? "Saved in keychain (unchanged)" : "If the key is encrypted"}
            />
          {/if}
        </div>
      {/if}
    </fieldset>
  {/if}

  {#if status}
    <p class={status.ok ? "ok" : "error-text"} role="status">{status.text}</p>
  {/if}

  <div class="actions">
    {#if saved}<button type="button" class="btn danger" onclick={remove}>Delete</button>{/if}
    <span class="spacer"></span>
    <button type="button" class="btn" onclick={test} disabled={busy !== ""}>{busy === "test" ? "Testing…" : "Test"}</button>
    <button type="button" class="btn" onclick={saveOnly} disabled={busy !== ""}>Save</button>
    <button type="submit" class="btn primary" disabled={busy !== ""}>{busy === "connect" ? "Connecting…" : "Connect"}</button>
  </div>
</form>

<style>
  .form {
    width: min(560px, 100%);
    margin: 0 auto;
    padding: 8px 28px 28px;
  }
  h1 {
    font-size: 18px;
    margin: 0 0 16px;
  }
  .kinds {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-bottom: 18px;
  }
  .kind {
    height: 28px;
    padding: 0 12px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--panel);
  }
  .kind.on {
    border-color: var(--accent);
    background: var(--selection);
  }
  .grid {
    display: grid;
    grid-template-columns: 96px 1fr;
    gap: 8px 12px;
    align-items: center;
  }
  .grid > label,
  .label {
    text-align: right;
    color: var(--muted);
  }
  .row {
    display: flex;
    gap: 8px;
    align-items: center;
    min-width: 0;
  }
  .grow {
    flex: 1;
  }
  .inline {
    color: var(--muted);
  }
  .port {
    width: 76px;
  }
  .narrow {
    width: 140px;
  }
  .colors {
    display: flex;
    gap: 4px;
  }
  .color {
    width: 16px;
    height: 16px;
    border-radius: 4px;
    border: 2px solid transparent;
    padding: 0;
  }
  .color.on {
    border-color: var(--text);
  }
  .note {
    font-size: 12px;
    margin: 14px 0 0 108px;
  }
  .ssh {
    margin: 18px 0 0;
    padding: 10px 0 0;
    border: 0;
    border-top: 1px solid var(--border);
  }
  .ssh legend {
    padding: 0 6px 0 0;
  }
  .ssh .grid {
    margin-top: 10px;
  }
  .check {
    display: inline-flex;
    gap: 6px;
    align-items: center;
  }
  .ok {
    color: #16a34a;
  }
  .actions {
    display: flex;
    gap: 8px;
    margin-top: 22px;
  }
  .spacer {
    flex: 1;
  }
</style>
