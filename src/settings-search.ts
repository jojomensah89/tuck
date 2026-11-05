/** Pure helpers for the settings window: the search filter and sidebar key handling. */

export interface SearchEntry {
  /** Unique row id in the settings document. */
  id: string;
  /** The sidebar view the row belongs to. */
  view: string;
  /** Everything a person might type to find the row: label, description, group, view, keywords. */
  text: string;
}

export interface SearchResult {
  /** Row ids that match, in document order. */
  ids: string[];
  /** Matching row count per view; views with no match are absent. */
  views: Map<string, number>;
}

/** Lower-case, strip accents and punctuation, collapse whitespace. */
export function normalizeText(value: string): string {
  return value
    .normalize('NFKD')
    .replace(/[̀-ͯ]/g, '')
    .toLowerCase()
    .replace(/[^\p{L}\p{N}]+/gu, ' ')
    .trim();
}

/** Split a query into distinct search tokens. An empty or blank query has none. */
export function queryTokens(query: string): string[] {
  const normalized = normalizeText(query);
  return normalized ? [...new Set(normalized.split(' '))] : [];
}

/**
 * A row matches when every token starts a word in its text, so "del" finds
 * "Expand delay" but "lay" does not, and "api key" needs both words.
 */
export function matchesTokens(text: string, tokens: readonly string[]): boolean {
  if (!tokens.length) return true;
  const words = normalizeText(text).split(' ');
  return tokens.every(token => words.some(word => word.startsWith(token)));
}

/** Filters rows by query. Rows in `unavailable` are not on screen, so they never match or count. */
export function filterSettings(
  entries: readonly SearchEntry[],
  query: string,
  unavailable: ReadonlySet<string> = new Set(),
): SearchResult {
  const tokens = queryTokens(query);
  const ids: string[] = [];
  const views = new Map<string, number>();
  for (const entry of entries) {
    if (unavailable.has(entry.id) || !matchesTokens(entry.text, tokens)) continue;
    ids.push(entry.id);
    views.set(entry.view, (views.get(entry.view) ?? 0) + 1);
  }
  return { ids, views };
}

/** Whether a row stays hidden: it has nothing to show, or a search is running and it did not match. */
export function isRowHidden(
  id: string,
  searching: boolean,
  matched: ReadonlySet<string>,
  unavailable: ReadonlySet<string>,
): boolean {
  return unavailable.has(id) || (searching && !matched.has(id));
}

/** Index of the sidebar item a key moves to, or null when the key does not navigate. Wraps at the ends. */
export function navTarget(current: number, key: string, count: number): number | null {
  if (count <= 0) return null;
  const from = current < 0 || current >= count ? 0 : current;
  switch (key) {
    case 'ArrowDown': return (from + 1) % count;
    case 'ArrowUp': return (from - 1 + count) % count;
    case 'Home': return 0;
    case 'End': return count - 1;
    default: return null;
  }
}

/** Summary line for a search, written for a status region. */
export function describeResult(result: SearchResult, query: string): string {
  const trimmed = query.trim();
  if (!trimmed) return '';
  const count = result.ids.length;
  if (!count) return `No settings match "${trimmed}".`;
  return `${count} ${count === 1 ? 'setting matches' : 'settings match'} "${trimmed}".`;
}
