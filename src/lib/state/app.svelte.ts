import { api, errorText } from "../api";
import { uid } from "../cells";
import type { ConnectionProfile, SessionInfo, TableInfo, TableRef } from "../types";

export type Tab =
  | { id: string; kind: "table"; table: TableRef; title: string; dirty: boolean }
  | { id: string; kind: "query"; title: string; sql: string; dirty: boolean }
  | { id: string; kind: "collection"; db: string; name: string; title: string; dirty: boolean }
  | { id: string; kind: "mongo-command"; title: string; text: string; dirty: boolean };

interface Confirm {
  message: string;
  detail?: string;
  ok: string;
  danger: boolean;
  resolve: (v: boolean) => void;
}

class AppState {
  connections = $state<ConnectionProfile[]>([]);
  session = $state<SessionInfo | null>(null);
  connecting = $state<string | null>(null);
  connectError = $state<string | null>(null);

  databases = $state<string[]>([]);
  schemas = $state<string[]>([]);
  schema = $state("");
  tables = $state<TableInfo[]>([]);
  tablesLoading = $state(false);
  sidebarError = $state<string | null>(null);

  tabs = $state<Tab[]>([]);
  activeTab = $state<string | null>(null);
  queryCount = 0;

  confirm = $state<Confirm | null>(null);

  async loadConnections() {
    this.connections = await api.listConnections();
  }

  ask(message: string, opts: { detail?: string; ok?: string; danger?: boolean } = {}): Promise<boolean> {
    return new Promise((resolve) => {
      this.confirm = { message, detail: opts.detail, ok: opts.ok ?? "OK", danger: opts.danger ?? false, resolve };
    });
  }

  answer(v: boolean) {
    this.confirm?.resolve(v);
    this.confirm = null;
  }

  async connect(id: string) {
    this.connecting = id;
    this.connectError = null;
    try {
      const s = await api.connect(id);
      this.session = s;
      this.tabs = [];
      this.activeTab = null;
      this.queryCount = 0;
      await this.loadSidebar();
    } catch (e) {
      this.connectError = errorText(e);
    } finally {
      this.connecting = null;
    }
  }

  async disconnect() {
    if (!this.session) return;
    if (this.tabs.some((t) => t.dirty)) {
      const ok = await this.ask("Discard unsaved changes and disconnect?", { ok: "Disconnect", danger: true });
      if (!ok) return;
    }
    const id = this.session.id;
    this.session = null;
    this.tabs = [];
    this.tables = [];
    this.schemas = [];
    this.databases = [];
    await api.disconnect(id).catch(() => {});
  }

  async loadSidebar() {
    const s = this.session;
    if (!s) return;
    this.sidebarError = null;
    if (s.kind === "redis") {
      this.databases = await api.listDatabases(s.id).catch(() => []);
      return;
    }
    try {
      const [schemas, databases] = await Promise.all([api.listSchemas(s.id), api.listDatabases(s.id)]);
      this.schemas = schemas;
      this.databases = databases;
      const firstUserDb = schemas.find((d) => !["admin", "config", "local"].includes(d));
      const preferred =
        { mysql: s.database, sqlite: "main", mssql: "dbo", mongodb: s.database || firstUserDb }[s.kind as string] ??
        "public";
      this.schema = schemas.includes(this.schema) ? this.schema : schemas.includes(preferred) ? preferred : (schemas[0] ?? "");
      await this.loadTables();
    } catch (e) {
      this.sidebarError = errorText(e);
    }
  }

  async loadTables() {
    const s = this.session;
    if (!s || !this.schema) {
      this.tables = [];
      return;
    }
    this.tablesLoading = true;
    this.sidebarError = null;
    try {
      this.tables = await api.listTables(s.id, this.schema);
    } catch (e) {
      this.sidebarError = errorText(e);
      this.tables = [];
    } finally {
      this.tablesLoading = false;
    }
  }

  async selectSchema(schema: string) {
    const s = this.session;
    if (!s) return;
    this.schema = schema;
    // On MySQL a schema is a database: make it the editor's default too.
    if (s.kind === "mysql" && schema !== s.database) {
      try {
        s.database = await api.switchDatabase(s.id, schema);
      } catch (e) {
        this.sidebarError = errorText(e);
      }
    }
    await this.loadTables();
  }

  async switchDatabase(db: string) {
    const s = this.session;
    if (!s || db === s.database) return;
    if (this.tabs.some((t) => t.dirty)) {
      const ok = await this.ask(`Discard unsaved changes and switch to ${db}?`, { ok: "Switch", danger: true });
      if (!ok) return;
    }
    try {
      s.database = await api.switchDatabase(s.id, db);
      this.tabs = this.tabs.filter((t) => t.kind === "query" || t.kind === "mongo-command");
      this.activeTab = this.tabs.at(-1)?.id ?? null;
      this.schema = "";
      await this.loadSidebar();
    } catch (e) {
      this.sidebarError = errorText(e);
    }
  }

  openTable(name: string) {
    if (this.session?.kind === "mongodb") {
      const existing = this.tabs.find((t) => t.kind === "collection" && t.db === this.schema && t.name === name);
      if (existing) {
        this.activeTab = existing.id;
        return;
      }
      const tab: Tab = { id: uid(), kind: "collection", db: this.schema, name, title: name, dirty: false };
      this.tabs.push(tab);
      this.activeTab = tab.id;
      return;
    }
    const table = { schema: this.schema, name };
    const existing = this.tabs.find(
      (t) => t.kind === "table" && t.table.schema === table.schema && t.table.name === table.name,
    );
    if (existing) {
      this.activeTab = existing.id;
      return;
    }
    const tab: Tab = { id: uid(), kind: "table", table, title: name, dirty: false };
    this.tabs.push(tab);
    this.activeTab = tab.id;
  }

  newQuery(sql = "") {
    this.queryCount += 1;
    if (this.session?.kind === "mongodb") {
      const tab: Tab = {
        id: uid(),
        kind: "mongo-command",
        title: `Command ${this.queryCount}`,
        text: sql || '{ "listCollections": 1, "nameOnly": true }',
        dirty: false,
      };
      this.tabs.push(tab);
      this.activeTab = tab.id;
      return;
    }
    const tab: Tab = { id: uid(), kind: "query", title: `Query ${this.queryCount}`, sql, dirty: false };
    this.tabs.push(tab);
    this.activeTab = tab.id;
  }

  async closeTab(id: string) {
    const i = this.tabs.findIndex((t) => t.id === id);
    if (i < 0) return;
    const tab = this.tabs[i];
    if ((tab.kind === "table" || tab.kind === "collection") && tab.dirty) {
      const ok = await this.ask(`Discard unsaved changes to ${tab.title}?`, { ok: "Discard", danger: true });
      if (!ok) return;
    }
    this.tabs.splice(i, 1);
    if (this.activeTab === id) this.activeTab = (this.tabs[i] ?? this.tabs[i - 1])?.id ?? null;
  }
}

export const app = new AppState();
