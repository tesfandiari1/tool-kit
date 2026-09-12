/// Last path component, for both POSIX and Windows-style separators.
export function basename(p: string) {
  return p.split(/[\\/]/).filter(Boolean).pop() ?? p;
}

/// `/Users/me/Desktop/a.md` -> `~/Desktop/a.md`.
///
/// macOS only, so the home directory is `/Users/<name>` and a regex beats a
/// host call. Unmatched, the full path shows.
export function tildePath(p: string) {
  return p.replace(/^\/Users\/[^/]+/, "~");
}

export function fmtElapsed(nowMs: number, startedSec: number) {
  const s = Math.max(0, Math.floor(nowMs / 1000) - startedSec);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/// History is scanned, not read: relative while that is the useful answer,
/// then a date.
export function fmtWhen(finishedSec: number, nowMs: number) {
  const s = Math.max(0, Math.floor(nowMs / 1000) - finishedSec);
  if (s < 90) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  if (s < 172800) return "yesterday";
  return new Date(finishedSec * 1000).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}
