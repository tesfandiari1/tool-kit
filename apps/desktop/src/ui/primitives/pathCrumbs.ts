/// A path as crumbs, eliding the middle rather than the end, which would drop
/// the file name. Its own module, or fast refresh breaks.
const MAX_CRUMBS = 4;

export function pathCrumbs(path: string): string[] {
  const parts = path.split("/").filter(Boolean);
  // A leading slash is a crumb, so the path still reads as absolute.
  if (path.startsWith("/")) parts.unshift("/");
  if (parts.length <= MAX_CRUMBS) return parts;
  return [parts[0], "…", ...parts.slice(-(MAX_CRUMBS - 2))];
}
