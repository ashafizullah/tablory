import { isBinary } from "./cells";
import type { Cell, DbKind } from "./types";

export function quoteIdent(kind: DbKind, name: string): string {
  if (kind === "mysql") return "`" + name.replace(/`/g, "``") + "`";
  if (kind === "mssql") return "[" + name.replace(/]/g, "]]") + "]";
  return '"' + name.replace(/"/g, '""') + '"';
}

function literal(kind: DbKind, c: Cell): string {
  if (c === null) return "NULL";
  if (typeof c === "number") return String(c);
  if (typeof c === "boolean") return kind === "postgres" ? (c ? "TRUE" : "FALSE") : c ? "1" : "0";
  if (isBinary(c)) {
    // Only the first bytes reach the grid: a longer value can't be written out.
    if (c.hex.length !== c.$bin * 2) return "NULL";
    if (kind === "postgres") return `'\\x${c.hex}'::bytea`;
    if (kind === "sqlite") return `X'${c.hex}'`;
    return "0x" + c.hex;
  }
  let s = String(c).replace(/'/g, "''");
  if (kind === "mysql") s = s.replace(/\\/g, "\\\\");
  return kind === "mssql" ? `N'${s}'` : `'${s}'`;
}

/** One INSERT per row; `table` is already quoted. */
export function insertStatements(kind: DbKind, table: string, columns: string[], rows: Cell[][]): string {
  const cols = columns.map((c) => quoteIdent(kind, c)).join(", ");
  return rows.map((r) => `INSERT INTO ${table} (${cols}) VALUES (${r.map((c) => literal(kind, c)).join(", ")});`).join("\n");
}

/** The table a simple `SELECT … FROM t` reads, as written, for INSERT output. */
export function tableFromSql(sql: string): string | null {
  const m = /\bfrom\s+((?:[\w$#@]+|"[^"]+"|`[^`]+`|\[[^\]]+\])(?:\s*\.\s*(?:[\w$#@]+|"[^"]+"|`[^`]+`|\[[^\]]+\]))*)/i.exec(sql);
  return m ? m[1].replace(/\s+/g, "") : null;
}
