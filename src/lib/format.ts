export function formatDuration(seconds: number): string {
  if (!seconds) return "";
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const s = Math.floor(seconds % 60);
  return h > 0
    ? `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`
    : `${m}:${String(s).padStart(2, "0")}`;
}

export function formatTotalDuration(seconds: number): string {
  if (!seconds) return "";
  const h = Math.floor(seconds / 3600);
  const m = Math.round((seconds % 3600) / 60);
  return h > 0 ? `${h} hr ${m} min` : `${m} min`;
}

export function formatBytes(value: number): string {
  if (!value) return "0 B";
  if (value < 1_048_576) return `${Math.max(1, Math.round(value / 1024))} KB`;
  if (value < 1_073_741_824) return `${(value / 1_048_576).toFixed(1)} MB`;
  return `${(value / 1_073_741_824).toFixed(2)} GB`;
}

export function formatSpeed(value: number): string {
  return value > 0 ? `${formatBytes(value)}/s` : "—";
}

export function formatCount(value: number | null | undefined): string {
  if (value == null) return "";
  return new Intl.NumberFormat("en", { notation: "compact", maximumFractionDigits: 1 }).format(value);
}

export function formatAge(days: number): string {
  if (days < 1) return "today";
  if (days < 30) return `${Math.round(days)}d`;
  if (days < 365) return `${Math.round(days / 30)}mo`;
  return `${(days / 365).toFixed(1)}y`;
}

export function kindLabel(kind: string): string {
  switch (kind) {
    case "ep": return "EP";
    case "single": return "Single";
    case "compile": return "Compilation";
    case "album": return "Album";
    default: return kind ? kind[0].toUpperCase() + kind.slice(1) : "Release";
  }
}

export function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  try { return JSON.stringify(error); } catch { return String(error); }
}

/** Strips edition decorations so provider searches match more releases. */
export function simplifyTitle(title: string): string {
  return title
    .replace(/[\(\[][^)\]]*(deluxe|edition|remaster|version|expanded|anniversary|bonus)[^)\]]*[\)\]]/gi, "")
    .replace(/\s+-\s+(deluxe|remaster|\d{4} remaster).*$/i, "")
    .replace(/[^\p{L}\p{N}\s'&]/gu, " ")
    .replace(/\s+/g, " ")
    .trim();
}

export function remoteFolder(filename: string): string {
  const normalized = filename.replace(/\//g, "\\");
  const separator = normalized.lastIndexOf("\\");
  return separator > 0 ? normalized.slice(0, separator) : "Shared music";
}

export function remoteTitle(filename: string): string {
  const normalized = filename.replace(/\//g, "\\");
  return normalized.slice(normalized.lastIndexOf("\\") + 1);
}
