import { api, errorText } from "../api";
import { uid } from "../cells";
import type { ConnectionProfile, RoutineInfo, SessionInfo, TableInfo, TableRef } from "../types";

export type Tab =
  | { id: string; kind: "table"; table: TableRef; title: string; dirty: boolean }
  | {
      id: string;
      kind: "query";
      title: string;
      sql: string;
      dirty: boolean;
      /** The .sql file it was opened from or saved to. */
      path: string | null;
      /** The text as last saved or opened, to tell if it changed. */
      saved: string;
    }
  | { id: string; kind: "collection"; db: string; name: string; title: string; dirty: boolean }
  | { id: string; kind: "mongo-command"; title: string; text: string; dirty: boolean };

type QueryTab = Extract<Tab, { kind: "query" }>;

/** The query text changed since it was last saved, opened or created. */
export const queryModified = (t: QueryTab) => t.sql !== t.saved;

const fileTitle = (path: string) => path.split(/[\\/]/).pop()!.replace(/\.sql$/i, "");

/** Open query tabs per connection, restored on the next connect. */
const QUERIES_KEY = (connectionId: string) => `tablory.queryTabs.${connectionId}`;
type StoredQuery = Pick<QueryTab, "title" | "sql" | "path" | "saved">;

interface Confirm {
  message: string;
  detail?: string;
  ok: string;
  /** A third choice, e.g. "Don't save". */
  alt?: string;
  danger: boolean;
  resolve: (v: boolean | "alt") => void;
}

class AppState {
  connections = $state<ConnectionProfile[]>([]);
  groups = $state<string[]>([]);
  groupColors = $state<Record<string, string>>({});
  session = $state<SessionInfo | null>(null);
  /** The connection manager, or the workspace with its connection tree. */
  view = $state<"connections" | "workspace">("connections");
  /** The current database is closed in the tree: its objects are unloaded. */
  dbClosed = $state(false);
  connecting = $state<string | null>(null);
  connectError = $state<string | null>(null);

  databases = $state<string[]>([]);
  schemas = $state<string[]>([]);
  schema = $state("");
  tables = $state<TableInfo[]>([]);
  routines = $state<RoutineInfo[]>([]);
  tablesLoading = $state(false);
  sidebarError = $state<string | null>(null);

  tabs = $state<Tab[]>([]);
  activeTab = $state<string | null>(null);
  queryCount = 0;

  confirm = $state<Confirm | null>(null);

  async loadConnections() {
    [this.connections, this.groups, this.groupColors] = await Promise.all([
      api.listConnections(),
      api.listGroups(),
      api.listGroupColors(),
    ]);
  }

  ask(message: string, opts: { detail?: string; ok?: string; danger?: boolean } = {}): Promise<boolean> {
    return new Promise((resolve) => {
      this.confirm = {
        message,
        detail: opts.detail,
        ok: opts.ok ?? "OK",
        danger: opts.danger ?? false,
        resolve: (v) => resolve(v === true),
      };
    });
  }

  /** Like `ask`, with a third button that resolves to "alt". */
  choose(message: string, opts: { detail?: string; ok: string; alt: string }): Promise<boolean | "alt"> {
    return new Promise((resolve) => {
      this.confirm = { message, detail: opts.detail, ok: opts.ok, alt: opts.alt, danger: false, resolve };
    });
  }

  answer(v: boolean | "alt") {
    this.confirm?.resolve(v);
    this.confirm = null;
  }

  /** Connects, replacing the current session once the new one is open. */
  async connect(id: string) {
    if (this.connecting) return;
    const old = this.session;
    if (old && this.tabs.some((t) => t.dirty)) {
      const ok = await this.ask(`Discard unsaved changes and leave ${old.name}?`, { ok: "Discard", danger: true });
      if (!ok) return;
    }
    this.connecting = id;
    this.connectError = null;
    try {
      const s = await api.connect(id);
      this.session = s;
      this.view = "workspace";
      this.dbClosed = false;
      this.tabs = [];
      this.activeTab = null;
      this.queryCount = 0;
      this.tables = [];
      this.routines = [];
      this.schemas = [];
      this.databases = [];
      this.schema = "";
      this.restoreQueries();
      if (old) api.disconnect(old.id).catch(() => {});
      await this.loadSidebar();
    } catch (e) {
      this.connectError = errorText(e);
    } finally {
      this.connecting = null;
    }
  }

  /** Closes the session; the workspace stays open on the connection tree. */
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
    this.routines = [];
    this.schemas = [];
    this.databases = [];
    await api.disconnect(id).catch(() => {});
  }

  /** Unloads the current database's objects and closes its table tabs. */
  async closeDatabase() {
    const isObject = (t: Tab) => t.kind === "table" || t.kind === "collection";
    if (this.tabs.some((t) => isObject(t) && t.dirty)) {
      const ok = await this.ask("Discard unsaved changes and close the database?", { ok: "Close", danger: true });
      if (!ok) return;
    }
    this.tabs = this.tabs.filter((t) => !isObject(t));
    if (!this.tabs.some((t) => t.id === this.activeTab)) this.activeTab = this.tabs.at(-1)?.id ?? null;
    this.tables = [];
    this.routines = [];
    this.sidebarError = null;
    this.dbClosed = true;
  }

  async openDatabase() {
    this.dbClosed = false;
    await this.loadTables();
  }

  async loadSidebar() {
    const s = this.session;
    if (!s) return;
    this.sidebarError = null;
    this.dbClosed = false;
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
      this.routines = [];
      return;
    }
    this.tablesLoading = true;
    this.sidebarError = null;
    try {
      // Routines are a bonus: a permission error there keeps the tables.
      [this.tables, this.routines] = await Promise.all([
        api.listTables(s.id, this.schema),
        api.listRoutines(s.id, this.schema).catch(() => []),
      ]);
    } catch (e) {
      this.sidebarError = errorText(e);
      this.tables = [];
      this.routines = [];
    } finally {
      this.tablesLoading = false;
    }
  }

  async selectSchema(schema: string) {
    const s = this.session;
    if (!s) return;
    this.schema = schema;
    this.dbClosed = false;
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
      this.dbClosed = false;
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

  /** Opens a routine's CREATE statement in a new query tab. */
  async openRoutine(r: RoutineInfo) {
    const s = this.session;
    if (!s) return;
    try {
      this.newQuery(await api.routineDefinition(s.id, this.schema, r), r.name);
    } catch (e) {
      this.sidebarError = errorText(e);
    }
  }

  newQuery(sql = "", title?: string) {
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
    const tab: Tab = {
      id: uid(),
      kind: "query",
      title: title ?? `Query ${this.queryCount}`,
      sql,
      dirty: false,
      path: null,
      saved: sql,
    };
    this.tabs.push(tab);
    this.activeTab = tab.id;
  }

  /** Writes a query tab to its file, or asks where first (`as`, or never saved). */
  async saveQuery(tab: QueryTab, as = false): Promise<boolean> {
    const sql = tab.sql;
    try {
      const path = await api.saveSqlFile(as ? null : tab.path, tab.title, sql);
      if (!path) return false;
      tab.path = path;
      tab.saved = sql;
      tab.title = fileTitle(path);
      return true;
    } catch (e) {
      await this.ask("Could not save the query.", { detail: errorText(e) });
      return false;
    }
  }

  /** Opens a .sql file in a query tab, or switches to it if already open. */
  async openQueryFile() {
    if (!this.session || this.session.kind === "mongodb" || this.session.kind === "redis") return;
    let file;
    try {
      file = await api.openSqlFile();
    } catch (e) {
      await this.ask("Could not open the file.", { detail: errorText(e) });
      return;
    }
    if (!file) return;
    const existing = this.tabs.find((t) => t.kind === "query" && t.path === file.path);
    if (existing) {
      this.activeTab = existing.id;
      return;
    }
    const tab: Tab = {
      id: uid(),
      kind: "query",
      title: fileTitle(file.path),
      sql: file.contents,
      dirty: false,
      path: file.path,
      saved: file.contents,
    };
    this.tabs.push(tab);
    this.activeTab = tab.id;
  }

  /** Remembers the open query tabs, text included, for this connection. */
  persistQueries() {
    const s = this.session;
    if (!s) return;
    const queries: StoredQuery[] = this.tabs
      .filter((t): t is QueryTab => t.kind === "query")
      .map(({ title, sql, path, saved }) => ({ title, sql, path, saved }));
    try {
      if (queries.length) localStorage.setItem(QUERIES_KEY(s.connection_id), JSON.stringify(queries));
      else localStorage.removeItem(QUERIES_KEY(s.connection_id));
    } catch {}
  }

  private restoreQueries() {
    const s = this.session;
    if (!s || s.kind === "redis" || s.kind === "mongodb") return;
    let queries: StoredQuery[] = [];
    try {
      queries = JSON.parse(localStorage.getItem(QUERIES_KEY(s.connection_id)) ?? "[]");
    } catch {}
    if (!Array.isArray(queries)) return;
    for (const q of queries) {
      this.tabs.push({
        id: uid(),
        kind: "query",
        title: q.title,
        sql: q.sql ?? "",
        dirty: false,
        path: q.path ?? null,
        saved: q.saved ?? "",
      });
      const n = /^Query (\d+)$/.exec(q.title);
      if (n) this.queryCount = Math.max(this.queryCount, Number(n[1]));
    }
    this.activeTab = this.tabs.at(-1)?.id ?? null;
  }

  async closeTab(id: string) {
    const i = this.tabs.findIndex((t) => t.id === id);
    if (i < 0) return;
    const tab = this.tabs[i];
    if ((tab.kind === "table" || tab.kind === "collection") && tab.dirty) {
      const ok = await this.ask(`Discard unsaved changes to ${tab.title}?`, { ok: "Discard", danger: true });
      if (!ok) return;
    }
    if (tab.kind === "query" && queryModified(tab)) {
      const choice = await this.choose(`Save changes to ${tab.title}?`, {
        detail: "Closing the tab loses changes that are not saved to a file.",
        ok: "Save",
        alt: "Don't save",
      });
      if (choice === false) return;
      if (choice === true && !(await this.saveQuery(tab))) return;
    }
    // The tab list may have changed while a dialog was open.
    const at = this.tabs.findIndex((t) => t.id === id);
    if (at < 0) return;
    this.tabs.splice(at, 1);
    if (this.activeTab === id) this.activeTab = (this.tabs[at] ?? this.tabs[at - 1])?.id ?? null;
  }
}

export const app = new AppState();
