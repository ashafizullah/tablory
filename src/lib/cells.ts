import type { Binary, Cell, DbKind } from "./types";

export const isBinary = (c: Cell): c is Binary => typeof c === "object" && c !== null && "$bin" in c;

/** One-line text for a grid cell. */
export function display(c: Cell): string {
  if (c === null) return "NULL";
  if (isBinary(c)) return `(binary ${c.$bin} bytes)`;
  if (typeof c === "string") return c.length > 300 ? c.slice(0, 300).replace(/\n/g, "↵") + "…" : c.replace(/\n/g, "↵");
  return String(c);
}

/** Text form sent back to the database (edits, primary keys). */
export function toText(c: Cell): string | null {
  if (c === null) return null;
  if (isBinary(c)) return "0x" + c.hex;
  return String(c);
}

/** Value used for copy / editing. */
export function editText(c: Cell): string {
  return c === null ? "" : isBinary(c) ? "" : String(c);
}

export const isNumeric = (c: Cell) => typeof c === "number";

export const kindLabel: Record<DbKind, string> = {
  postgres: "PostgreSQL",
  mysql: "MySQL / MariaDB",
  sqlite: "SQLite",
  mssql: "SQL Server",
  redis: "Redis",
  mongodb: "MongoDB",
};

export const kindShort: Record<DbKind, string> = {
  postgres: "PG",
  mysql: "My",
  sqlite: "SL",
  mssql: "MS",
  redis: "Rd",
  mongodb: "Mg",
};

export function uid() {
  return Math.random().toString(36).slice(2, 10);
}
