export type DbKind = "postgres" | "mysql" | "sqlite" | "mssql" | "redis" | "mongodb";
export type SslMode = "disable" | "prefer" | "require";

export interface SshConfig {
  enabled: boolean;
  host: string;
  port: number;
  user: string;
  auth: "password" | "key";
  key_path: string;
}

/** Guard rails for SQL connections. */
export type Safety = "normal" | "production" | "readonly";

export interface ConnectionProfile {
  id: string;
  name: string;
  /** Folder in the connection list; "" means ungrouped. */
  group: string;
  kind: DbKind;
  color: string;
  host: string;
  port: number;
  user: string;
  database: string;
  file: string;
  ssl_mode: SslMode;
  ssh: SshConfig;
  uri: string;
  auth_source: string;
  /** SQL Server: log in as the current Windows user. */
  windows_auth: boolean;
  safety: Safety;
}

/** `null` keeps the stored secret; "" clears it. */
export interface Secrets {
  password: string | null;
  ssh_password: string | null;
  ssh_passphrase: string | null;
}

export interface SessionInfo {
  id: string;
  connection_id: string;
  name: string;
  kind: DbKind;
  color: string;
  database: string;
  safety: Safety;
}

export interface TableRef {
  schema: string;
  name: string;
}

export interface TableInfo {
  name: string;
  kind: "table" | "view";
}

export interface RoutineInfo {
  name: string;
  kind: "function" | "procedure";
  /** Lookup key for the definition (object id on PostgreSQL / SQL Server). */
  id: string;
}

export interface ColumnInfo {
  name: string;
  data_type: string;
  nullable: boolean;
  default: string | null;
  primary_key: boolean;
}

export interface TableStructure {
  columns: ColumnInfo[];
  indexes: { name: string; columns: string[]; unique: boolean; primary: boolean }[];
  foreign_keys: { name: string; columns: string[]; ref_table: string; ref_columns: string[] }[];
}

export interface Binary {
  $bin: number;
  hex: string;
}
export type Cell = null | boolean | number | string | Binary;

export interface ColumnMeta {
  name: string;
  type_name: string;
}

export interface ResultSet {
  columns: ColumnMeta[];
  rows: Cell[][];
  truncated: boolean;
}

export interface StatementResult {
  result: ResultSet | null;
  rows_affected: number;
}

export interface ExecuteResult {
  statements: StatementResult[];
  duration_ms: number;
}

export interface Filter {
  column: string;
  op: string;
  value: string | null;
}

export interface RowsRequest {
  table: TableRef;
  filters: Filter[];
  raw_where: string | null;
  sort: { column: string; desc: boolean } | null;
  limit: number;
  offset: number;
}

export interface ColValue {
  column: string;
  value: string | null;
}

export type RowChange =
  | { type: "update"; key: ColValue[]; values: ColValue[] }
  | { type: "insert"; values: ColValue[] }
  | { type: "delete"; key: ColValue[] };

export interface RedisKey {
  key: string;
  kind: string;
}

export interface RedisValue {
  key: string;
  kind: string;
  ttl: number;
  length: number;
  value: unknown;
  truncated: boolean;
}

export interface FindResult {
  docs: Record<string, unknown>[];
  texts: string[];
}
