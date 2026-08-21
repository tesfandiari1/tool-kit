import { Segmented } from "@ui";
import type { View } from "@/app/types";

const ITEMS = [
  { value: "library", label: "Library" },
  { value: "run", label: "Run" },
  { value: "history", label: "History" },
] satisfies { value: View; label: string }[];

/// The three surfaces the left column holds, in the title bar so conversion and
/// history stay one click away. Settings is not among them: it is a sheet over
/// the whole window, reached from the sidebar gear or ⌘,.
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
