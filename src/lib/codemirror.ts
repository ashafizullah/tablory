import { EditorView, keymap } from "@codemirror/view";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { tags as t } from "@lezer/highlight";
import { MSSQL, MySQL, PostgreSQL, SQLite, type SQLDialect } from "@codemirror/lang-sql";
import type { DbKind } from "./types";

export { keymap };

export const dialectFor = (kind: DbKind): SQLDialect =>
  kind === "mysql" ? MySQL : kind === "sqlite" ? SQLite : kind === "mssql" ? MSSQL : PostgreSQL;

/** Colors come from CSS variables so the editor follows the app theme. */
export const theme = EditorView.theme({
  "&": { height: "100%", fontSize: "13px", backgroundColor: "var(--panel)", color: "var(--text)" },
  ".cm-scroller": { fontFamily: "var(--mono)", lineHeight: "1.55" },
  ".cm-content": { caretColor: "var(--text)" },
  // drawSelection hides the native caret and draws its own, black by default.
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--text)", borderLeftWidth: "1.5px" },
  ".cm-gutters": { backgroundColor: "var(--bg)", color: "var(--muted)", border: "none", borderRight: "1px solid var(--border)" },
  ".cm-activeLine": { backgroundColor: "transparent" },
  ".cm-activeLineGutter": { backgroundColor: "var(--hover)" },
  "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection": {
    backgroundColor: "var(--selection) !important",
  },
  "&.cm-focused": { outline: "none" },
  ".cm-tooltip": { backgroundColor: "var(--panel)", border: "1px solid var(--border)", color: "var(--text)" },
  ".cm-tooltip-autocomplete > ul > li[aria-selected]": { backgroundColor: "var(--accent)", color: "var(--accent-text)" },
  ".cm-matchingBracket": { backgroundColor: "var(--hover)", outline: "none" },
  ".cm-running": { backgroundColor: "var(--edited)" },
});

export const highlight = syntaxHighlighting(
  HighlightStyle.define([
    { tag: [t.keyword, t.operatorKeyword, t.modifier], color: "var(--syn-keyword)", fontWeight: "600" },
    // Built-in functions such as COUNT or GETDATE.
    { tag: t.standard(t.name), color: "var(--syn-function)" },
    { tag: t.typeName, color: "var(--syn-type)" },
    { tag: t.string, color: "var(--syn-string)" },
    // lang-sql tags quoted identifiers ("name", `name`, [name]) as special strings.
    { tag: t.special(t.string), color: "var(--syn-ident)" },
    { tag: t.special(t.name), color: "var(--syn-variable)" },
    { tag: [t.number, t.bool, t.null], color: "var(--syn-number)" },
    { tag: t.operator, color: "var(--syn-operator)" },
    { tag: [t.comment, t.lineComment, t.blockComment], color: "var(--syn-comment)", fontStyle: "italic" },
  ]),
);
