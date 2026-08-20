/// Split a path into the crumbs to show, eliding the middle when there are too
/// many.
///
/// Elides the middle rather than truncating the end, which would drop the file
/// name, the part always worth reading.
///
/// Its own module so `Path.tsx` exports only a component and fast refresh keeps
/// working.
const MAX_CRUMBS = 4;

export function pathCrumbs(path: string): string[] {
  const parts = path.split("/").filter(Boolean);
  // A leading slash becomes a crumb, so an absolute path still reads as
  // absolute once the middle is gone.
  if (path.startsWith("/")) parts.unshift("/");
  if (parts.length <= MAX_CRUMBS) return parts;
  return [parts[0], "…", ...parts.slice(-(MAX_CRUMBS - 2))];
}
