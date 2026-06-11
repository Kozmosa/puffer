const CONTACT_ID_PREFIXES = new Set([
  "telegram",
  "telegram-user-id",
  "google",
  "slack",
  "discord",
  "matrix",
  "lark"
]);
const TYPOGRAPHIC_DASHES = /[\u2010-\u2015\u2212]/g;

export function normalizeContactId(value: unknown): string | null {
  const trimmed = String(value).trim();
  const atIndex = trimmed.indexOf("@");
  if (atIndex <= 0) return null;
  const prefix = trimmed.slice(0, atIndex).trim().replace(TYPOGRAPHIC_DASHES, "-").toLowerCase();
  if (!CONTACT_ID_PREFIXES.has(prefix)) return null;
  let suffix = trimmed.slice(atIndex + 1).trim().replace(/^@+/, "");
  if (!suffix || /[\s\x00-\x1f\x7f]/.test(suffix)) return null;
  if (prefix === "telegram") {
    suffix = suffix.toLowerCase();
    if (/^\d+$/.test(suffix) || !/^[a-z0-9_]+$/.test(suffix)) return null;
  }
  if (prefix === "telegram-user-id") {
    if (!/^\d+$/.test(suffix)) return null;
    suffix = suffix.replace(/^0+/, "") || "0";
    if (suffix === "0") return null;
  }
  if (prefix === "google") suffix = suffix.toLowerCase();
  return `${prefix}@${suffix}`;
}

export function normalizeContactIds(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return Array.from(
    new Set(value.map(normalizeContactId).filter((id): id is string => id !== null))
  ).sort();
}

export function contactIdsKey(value: unknown): string {
  return normalizeContactIds(value).join("\n");
}
