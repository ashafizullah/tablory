import { splitStatements } from "./sqlsplit";

export interface StatementCheck {
  sql: string;
  /** The statement's main verb, lowercased ("select", "delete", …). */
  verb: string;
  /** The word after the verb ("transaction" in START TRANSACTION). */
  next: string;
  /** Changes data or schema (or may: procedure calls count). */
  writes: boolean;
  /** Why the statement is risky enough to confirm, if it is. */
  danger: string | null;
}

/** Verbs that only read or set session state. */
const READ = new Set([
  "select", "show", "describe", "desc", "explain", "values", "table", "use", "set", "declare",
  "print", "begin", "start", "commit", "rollback", "savepoint", "release", "go", "pragma", "with",
]);

/**
 * Blanks out comments, string literals and quoted identifiers (keeping their
 * length) so keywords inside them are not mistaken for real ones.
 */
function code(sql: string, backslashEscapes: boolean): string {
  let out = "";
  let i = 0;
  const n = sql.length;
  const blank = (to: number) => {
    out += sql.slice(i, to).replace(/[^\n]/g, " ");
    i = to;
  };
  while (i < n) {
    const ch = sql[i];
    const next = sql[i + 1];
    if ((ch === "-" && next === "-") || (ch === "#" && backslashEscapes)) {
      const end = sql.indexOf("\n", i);
      blank(end < 0 ? n : end);
    } else if (ch === "/" && next === "*") {
      const end = sql.indexOf("*/", i + 2);
      blank(end < 0 ? n : end + 2);
    } else if (ch === "'" || ch === '"' || ch === "`" || ch === "[") {
      const close = ch === "[" ? "]" : ch;
      let j = i + 1;
      while (j < n) {
        if (backslashEscapes && ch === "'" && sql[j] === "\\") {
          j += 2;
          continue;
        }
        if (sql[j] === close) {
          if (sql[j + 1] === close) {
            j += 2;
            continue;
          }
          break;
        }
        j++;
      }
      // Keep quoted names as a placeholder word so "DELETE FROM [t]" still parses.
      const end = Math.min(j + 1, n);
      out += ch === "'" ? " ".repeat(end - i) : "x" + " ".repeat(end - i - 1);
      i = end;
    } else if (ch === "$") {
      const m = /^\$[A-Za-z_]*\$/.exec(sql.slice(i));
      if (m) {
        const end = sql.indexOf(m[0], i + m[0].length);
        blank(end < 0 ? n : end + m[0].length);
      } else {
        out += ch;
        i++;
      }
    } else {
      out += ch;
      i++;
    }
  }
  return out.toLowerCase();
}

/** Words outside any parentheses, with their offsets. */
function topLevelWords(text: string): { word: string; at: number }[] {
  const words: { word: string; at: number }[] = [];
  let depth = 0;
  const re = /[()]|[a-z_][a-z0-9_$#@]*/g;
  for (let m = re.exec(text); m; m = re.exec(text)) {
    if (m[0] === "(") depth++;
    else if (m[0] === ")") depth = Math.max(0, depth - 1);
    else if (depth === 0) words.push({ word: m[0], at: m.index });
  }
  return words;
}

export function checkStatement(sql: string, backslashEscapes = false): StatementCheck {
  const words = topLevelWords(code(sql, backslashEscapes)).map((w) => w.word);
  let verb = words[0] ?? "";
  let rest = words;
  // A CTE: the statement's verb is the first one after the WITH list.
  if (verb === "with") {
    const i = words.findIndex((w, k) => k > 0 && ["select", "insert", "update", "delete", "merge"].includes(w));
    verb = i < 0 ? "select" : words[i];
    rest = i < 0 ? words : words.slice(i);
  }
  // EXPLAIN ANALYZE really runs the statement.
  if (verb === "explain") {
    if (!rest.includes("analyze") && !rest.includes("analyse")) return { sql, verb, next: "", writes: false, danger: null };
    const i = rest.findIndex((w) => ["insert", "update", "delete", "merge"].includes(w));
    if (i < 0) return { sql, verb, next: "", writes: false, danger: null };
    verb = rest[i];
    rest = rest.slice(i);
  }
  const has = (w: string) => rest.includes(w);
  const next = rest[1] ?? "";

  let writes: boolean;
  if (verb === "select") writes = has("into"); // SELECT … INTO creates a table
  else if (verb === "pragma") writes = sql.includes("=");
  else writes = verb !== "" && !READ.has(verb);

  let danger: string | null = null;
  if ((verb === "delete" || verb === "update") && !has("where")) {
    danger = `${verb.toUpperCase()} without WHERE changes every row in the table`;
  } else if (verb === "truncate") {
    danger = "TRUNCATE removes every row in the table";
  } else if (verb === "drop") {
    const what = rest.find((w) => w !== "drop" && w !== "temporary" && w !== "if") ?? "object";
    danger = `DROP ${what.toUpperCase()} deletes it permanently`;
  }
  return { sql, verb, next, writes, danger };
}

/**
 * A `SELECT COUNT(*)` over the rows a simple UPDATE or DELETE would touch,
 * or null when the statement is too involved to rewrite safely (joins,
 * USING/FROM lists, TOP/LIMIT, CTEs, cursors).
 */
export function countSql(sql: string, backslashEscapes = false): string | null {
  const masked = code(sql, backslashEscapes);
  const words = topLevelWords(masked);
  const at = (w: string, from = 0) => words.findIndex((x, i) => i >= from && x.word === w);
  const end = (w: { word: string; at: number }) => w.at + w.word.length;
  const verb = words[0]?.word;
  if (verb !== "delete" && verb !== "update") return null;

  let targetFrom: number;
  let targetTo: number;
  const whereIdx = at("where");
  if (verb === "delete") {
    const f = words[1]?.word === "from" ? 1 : 0;
    // DELETE t FROM t JOIN … (SQL Server, MySQL) has a second FROM.
    if (at("from", f + 1) >= 0 || at("using") >= 0) return null;
    targetFrom = end(words[f]);
    targetTo = whereIdx >= 0 ? words[whereIdx].at : sql.length;
  } else {
    const set = at("set");
    if (set < 0) return null;
    if (at("from", set) >= 0) return null;
    targetFrom = end(words[0]);
    targetTo = words[set].at;
  }
  const target = sql.slice(targetFrom, targetTo).trim();
  const targetWords = topLevelWords(masked.slice(targetFrom, targetTo)).map((w) => w.word);
  const blocked = ["join", "top", "low_priority", "ignore", "quick", "output", "limit", "order", "returning"];
  if (!target || targetWords.some((w) => blocked.includes(w)) || target.includes(",")) return null;

  let cond = "";
  if (whereIdx >= 0) {
    const stops = ["order", "limit", "returning", "output", "option"];
    const stop = words.findIndex((w, i) => i > whereIdx && stops.includes(w.word));
    cond = sql.slice(end(words[whereIdx]), stop >= 0 ? words[stop].at : sql.length).trim();
    const cut = stop >= 0 && words[stop].word !== "returning" && words[stop].word !== "output"; // LIMIT/ORDER BY
    if (!cond || cut || /^current\s+of\b/i.test(cond)) return null;
  }
  return `SELECT COUNT(*) FROM ${target}${cond ? ` WHERE ${cond}` : ""}`;
}

export function checkScript(sql: string, backslashEscapes = false): StatementCheck[] {
  return splitStatements(sql, backslashEscapes).map((s) => checkStatement(sql.slice(s.from, s.to), backslashEscapes));
}

/** One line per statement for a confirm dialog. */
export function preview(sql: string, max = 120): string {
  const one = sql.replace(/\s+/g, " ").trim();
  return one.length > max ? one.slice(0, max - 1) + "…" : one;
}
