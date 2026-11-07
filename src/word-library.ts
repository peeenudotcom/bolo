export type WordFields = { dictionary: string; corrections: string; snippets: string };
type Pair = { trigger: string; replacement: string };
export type WordLibrary = { format: "bolo-words"; version: 1; dictionary: string[]; corrections: Pair[]; snippets: Pair[] };
const MAX_ITEMS = 1000;
const name = (s: string) => s.trim().toLocaleLowerCase();
function text(value: unknown, max: number, multiline = false): string {
  if (typeof value !== "string" || !value.trim() || value.length > max || (!multiline && /[\r\n]/.test(value)))
    throw new Error("Each entry needs non-empty text within the supported length.");
  if (/[\u0000-\u0008\u000b\u000c\u000e-\u001f]/.test(value))
    throw new Error("Control characters are not allowed in word libraries.");
  return value;
}
export function parseLibrary(value: unknown): WordLibrary {
  if (!value || typeof value !== "object") throw new Error("Choose a Bolo word-library JSON file.");
  const v = value as Record<string, unknown>;
  if (v.format !== "bolo-words" || v.version !== 1) throw new Error("Unsupported word-library format or version.");
  for (const key of ["dictionary", "corrections", "snippets"])
    if (!Array.isArray(v[key]) || v[key].length > MAX_ITEMS) throw new Error("Each library section supports up to 1,000 entries.");
  const pairs = (key: "corrections" | "snippets") => (v[key] as unknown[]).map(item => {
    if (!item || typeof item !== "object") throw new Error("Invalid library entry.");
    const p = item as Record<string, unknown>;
    const trigger = text(p.trigger, 120).trim();
    if (trigger.includes("=>") || !/[\p{L}\p{N}]/u.test(trigger)) throw new Error("Triggers need a word or number and cannot contain =>.");
    const replacement = text(p.replacement, key === "snippets" ? 4000 : 120, key === "snippets");
    return { trigger, replacement: key === "snippets" ? replacement.replace(/\r\n?/g, "\n") : replacement.trim() };
  });
  return { format: "bolo-words", version: 1,
    dictionary: (v.dictionary as unknown[]).map(x => text(x, 120).trim()),
    corrections: pairs("corrections"), snippets: pairs("snippets") };
}
export function libraryFromFields(fields: WordFields): WordLibrary {
  const pairs = (s: string, multiline: boolean) => s.split(/\r?\n/).filter(l => l.trim()).map((line, i) => {
    const at = line.indexOf("=>");
    if (at < 0) throw new Error("Fix the missing => on line " + (i + 1) + " before exporting.");
    const replacement = line.slice(at + 2).trim();
    return { trigger: line.slice(0, at).trim(), replacement: multiline ? replacement.replace(/\\n/g, "\n") : replacement };
  });
  return parseLibrary({ format: "bolo-words", version: 1,
    dictionary: fields.dictionary.split(/\r?\n/).map(s => s.trim()).filter(Boolean),
    corrections: pairs(fields.corrections, false), snippets: pairs(fields.snippets, true) });
}
export function mergeLibrary(fields: WordFields, incoming: WordLibrary) {
  const result = { ...fields };
  let added = 0, skipped = 0;
  const conflicts: string[] = [];
  const append = (key: keyof WordFields, line: string) => {
    result[key] += (result[key] && !result[key].endsWith("\n") ? "\n" : "") + line;
    added++;
  };
  const words = new Set(fields.dictionary.split(/\r?\n/).map(name));
  for (const word of incoming.dictionary) {
    if (words.has(name(word))) { skipped++; continue; }
    words.add(name(word)); append("dictionary", word);
  }
  for (const key of ["corrections", "snippets"] as const) {
    const existing = new Map<string, string>();
    for (const line of fields[key].split(/\r?\n/)) {
      const at = line.indexOf("=>");
      if (at >= 0) existing.set(name(line.slice(0, at)), line.slice(at + 2).trim());
    }
    for (const p of incoming[key]) {
      const trigger = name(p.trigger), replacement = p.replacement.replace(/\n/g, "\\n");
      if (existing.has(trigger)) {
        skipped++;
        if (existing.get(trigger) !== replacement) conflicts.push(p.trigger);
      } else { existing.set(trigger, replacement); append(key, p.trigger + " => " + replacement); }
    }
  }
  return { fields: result, added, skipped, conflicts };
}
