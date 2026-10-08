/** Splits SQL into statements, skipping semicolons inside strings, quoted
 *  identifiers, comments and Postgres dollar-quoted bodies. */
export function splitStatements(sql: string, backslashEscapes = false): { from: number; to: number }[] {
  const out: { from: number; to: number }[] = [];
  let start = 0;
  let i = 0;
  const n = sql.length;
  while (i < n) {
    const ch = sql[i];
    const next = sql[i + 1];
    if (ch === "-" && next === "-") {
      while (i < n && sql[i] !== "\n") i++;
    } else if (ch === "#" && sql.slice(i, i + 2) !== "#>") {
      // MySQL line comment; "#>" is a Postgres JSON operator.
      while (i < n && sql[i] !== "\n") i++;
    } else if (ch === "/" && next === "*") {
      const end = sql.indexOf("*/", i + 2);
      i = end < 0 ? n : end + 2;
      continue;
    } else if (ch === "'" || ch === '"' || ch === "`") {
      i++;
      while (i < n) {
        if (backslashEscapes && sql[i] === "\\" && ch === "'") {
          i += 2;
          continue;
        }
        if (sql[i] === ch) {
          if (sql[i + 1] === ch) {
            i += 2;
            continue;
          }
          break;
        }
        i++;
      }
    } else if (ch === "$") {
      const m = /^\$[A-Za-z_]*\$/.exec(sql.slice(i));
      if (m) {
        const end = sql.indexOf(m[0], i + m[0].length);
        i = end < 0 ? n : end + m[0].length;
        continue;
      }
    } else if (ch === ";") {
      push(start, i);
      start = i + 1;
    }
    i++;
  }
  push(start, n);
  return out;

  function push(from: number, to: number) {
    const text = sql.slice(from, to);
    if (text.replace(/--[^\n]*|\/\*[\s\S]*?\*\//g, "").trim() === "") return;
    // Trim surrounding whitespace so ranges match what the user sees.
    const lead = text.length - text.trimStart().length;
    const trail = text.length - text.trimEnd().length;
    out.push({ from: from + lead, to: to - trail });
  }
}

/** The statement containing (or just before) `pos`. */
export function statementAt(sql: string, pos: number, backslashEscapes = false): { from: number; to: number } | null {
  const all = splitStatements(sql, backslashEscapes);
  let best: { from: number; to: number } | null = null;
  for (const s of all) {
    if (s.from <= pos + 1) best = s;
    if (pos <= s.to + 1 && pos >= s.from) return s;
  }
  return best;
}
