import { invoke } from "@tauri-apps/api/core";
import type {
  ConnectionProfile,
  ExecuteResult,
  FindResult,
  RedisKey,
  RedisValue,
  ResultSet,
  RoutineInfo,
  RowChange,
  RowsRequest,
  Secrets,
  SessionInfo,
  TableInfo,
  TableRef,
  TableStructure,
} from "./types";

export { invoke };

export const isMac = navigator.userAgent.includes("Mac");

export const api = {
  listConnections: () => invoke<ConnectionProfile[]>("list_connections"),
  saveConnection: (profile: ConnectionProfile, secrets: Secrets) =>
    invoke<ConnectionProfile>("save_connection", { profile, secrets }),
  listGroups: () => invoke<string[]>("list_groups"),
  listGroupColors: () => invoke<Record<string, string>>("list_group_colors"),
  setGroupColor: (name: string, color: string) => invoke<void>("set_group_color", { name, color }),
  createGroup: (name: string) => invoke<string>("create_group", { name }),
  renameGroup: (from: string, to: string) => invoke<void>("rename_group", { from, to }),
  deleteGroup: (name: string) => invoke<void>("delete_group", { name }),
  moveConnection: (id: string, group: string) => invoke<void>("move_connection", { id, group }),
  deleteConnection: (id: string) => invoke<void>("delete_connection", { id }),
  testConnection: (profile: ConnectionProfile, secrets: Secrets) =>
    invoke<string>("test_connection", { profile, secrets }),
  connect: (id: string) => invoke<SessionInfo>("connect", { id }),
  disconnect: (session: string) => invoke<void>("disconnect", { session }),
  switchDatabase: (session: string, database: string) =>
    invoke<string>("switch_database", { session, database }),
  listDatabases: (session: string) => invoke<string[]>("list_databases", { session }),
  listSchemas: (session: string) => invoke<string[]>("list_schemas", { session }),
  listTables: (session: string, schema: string) => invoke<TableInfo[]>("list_tables", { session, schema }),
  listRoutines: (session: string, schema: string) => invoke<RoutineInfo[]>("list_routines", { session, schema }),
  routineDefinition: (session: string, schema: string, r: RoutineInfo) =>
    invoke<string>("routine_definition", { session, schema, kind: r.kind, id: r.id }),
  tableStructure: (session: string, table: TableRef) =>
    invoke<TableStructure>("table_structure", { session, table }),
  fetchRows: (session: string, request: RowsRequest) => invoke<ResultSet>("fetch_rows", { session, request }),
  countRows: (session: string, request: RowsRequest) => invoke<number>("count_rows", { session, request }),
  execute: (session: string, sql: string, maxRows: number, queryId: string) =>
    invoke<ExecuteResult>("execute", { session, sql, maxRows, queryId }),
  cancelQuery: (session: string, queryId: string) => invoke<void>("cancel_query", { session, queryId }),
  previewChanges: (session: string, table: TableRef, changes: RowChange[]) =>
    invoke<string>("preview_changes", { session, table, changes }),
  applyChanges: (session: string, table: TableRef, changes: RowChange[]) =>
    invoke<number>("apply_changes", { session, table, changes }),
  importNavicat: () => invoke<{ imported: number; skipped: string[] } | null>("import_navicat"),
  pickFile: (create: boolean) => invoke<string | null>("pick_file", { create }),
  saveSqlFile: (path: string | null, name: string, contents: string) =>
    invoke<string | null>("save_sql_file", { path, name, contents }),
  openSqlFile: () => invoke<{ path: string; contents: string } | null>("open_sql_file"),

  redisScan: (session: string, cursor: string, pattern: string, count: number) =>
    invoke<{ cursor: string; keys: RedisKey[] }>("redis_scan", { session, cursor, pattern, count }),
  redisGet: (session: string, key: string) => invoke<RedisValue>("redis_get", { session, key }),
  redis: (session: string, ...args: string[]) => invoke<unknown>("redis_command", { session, args }),
  redisLine: (session: string, line: string) => invoke<unknown>("redis_command", { session, line }),

  mongoFind: (session: string, db: string, collection: string, filter: string, sort: string, skip: number, limit: number) =>
    invoke<FindResult>("mongo_find", { session, db, collection, filter, sort, skip, limit }),
  mongoCount: (session: string, db: string, collection: string, filter: string) =>
    invoke<number>("mongo_count", { session, db, collection, filter }),
  mongoInsert: (session: string, db: string, collection: string, doc: string) =>
    invoke<unknown>("mongo_insert", { session, db, collection, doc }),
  mongoReplace: (session: string, db: string, collection: string, id: string, doc: string) =>
    invoke<void>("mongo_replace", { session, db, collection, id, doc }),
  mongoDelete: (session: string, db: string, collection: string, ids: string[]) =>
    invoke<number>("mongo_delete", { session, db, collection, ids }),
  mongoCommand: (session: string, db: string, command: string) =>
    invoke<unknown>("mongo_command", { session, db, command }),
};

/** Tauri rejects with the Rust error string. */
export const errorText = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : String(e));
