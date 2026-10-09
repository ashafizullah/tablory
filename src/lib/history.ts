/** Queries run per connection, newest first, kept in this browser profile. */

export interface HistoryEntry {
  sql: string;
  /** Epoch milliseconds. */
  at: number;
  database: string;
  ms: number;
  /** Error text when the run failed. */
  error?: string;
  /** Rows returned by the last result, or affected when nothing returned rows. */
  rows?: number;
}

const MAX = 500;
const key = (connectionId: string) => `tablory.history.${connectionId}`;

export function loadHistory(connectionId: string): HistoryEntry[] {
  try {
    const v = JSON.parse(localStorage.getItem(key(connectionId)) ?? "[]");
    return Array.isArray(v) ? v : [];
  } catch {
    return [];
  }
}

function save(connectionId: string, entries: HistoryEntry[]) {
  try {
    localStorage.setItem(key(connectionId), JSON.stringify(entries));
  } catch {
    // Storage full: keep the newer half.
    try {
      localStorage.setItem(key(connectionId), JSON.stringify(entries.slice(0, entries.length >> 1)));
    } catch {}
  }
}

/** Adds a run; rerunning the newest query just updates it. */
export function addHistory(connectionId: string, entry: HistoryEntry): HistoryEntry[] {
  const all = loadHistory(connectionId);
  if (all[0]?.sql === entry.sql && all[0]?.database === entry.database) all.shift();
  all.unshift(entry);
  const kept = all.slice(0, MAX);
  save(connectionId, kept);
  return kept;
}

export function removeHistory(connectionId: string, at: number): HistoryEntry[] {
  const kept = loadHistory(connectionId).filter((e) => e.at !== at);
  save(connectionId, kept);
  return kept;
}

export function clearHistory(connectionId: string) {
  try {
    localStorage.removeItem(key(connectionId));
  } catch {}
}
