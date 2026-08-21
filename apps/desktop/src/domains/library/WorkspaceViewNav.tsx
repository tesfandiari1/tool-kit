import { Segmented } from "@ui";
import type { View } from "@/app/types";

const ITEMS = [
  { value: "library", label: "Library" },
  { value: "run", label: "Run" },
  { value: "settings", label: "Settings" },
  { value: "history", label: "History" },
] satisfies { value: View; label: string }[];

/// The workspace's four views, in the title bar so conversion, settings, and
/// history stay one click away without duplicating Settings in the corner.
export function WorkspaceViewNav({
  view,
  onView,
}: {
  view: View;
  onView: (view: View) => void;
}) {
  return (
    <Segmented
      className="bar-nav"
      size="sm"
      label="Workspace"
      value={view}
      onChange={onView}
      options={ITEMS}
    />
  );
}
