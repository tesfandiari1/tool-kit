/// The design system's entire public surface. Import from `@ui`, never deeper.
///
/// One hard rule, enforced by ESLint: nothing under `src/ui` may import from
/// `@/domains`, `@/app`, `@/platform` or `@tauri-apps/*`. See UI.md.

import "./base.css";

export { cx } from "./cx";

export { Label, Display, Text, Meta } from "./primitives/Text";
export type {
  LabelProps,
  LabelTone,
  DisplayProps,
  DisplaySize,
  TextProps,
  TextSize,
  TextTone,
  MetaProps,
  MetaSize,
  MetaTone,
} from "./primitives/Text";

export { Button } from "./primitives/Button";
export type { ButtonProps, ButtonVariant, ButtonSize } from "./primitives/Button";

export { Panel, Well, Divider } from "./primitives/Surface";
export type { PanelProps, WellProps, DividerProps } from "./primitives/Surface";

export { Badge, StatusDot } from "./primitives/Badge";
export type { BadgeProps, Tone, StatusDotProps, DotTone } from "./primitives/Badge";

export { Tabs } from "./primitives/Tabs";
export type { TabsProps, TabItem } from "./primitives/Tabs";

/// A file path as crumbs.
export { Path } from "./primitives/Path";
export type { PathProps } from "./primitives/Path";

export { SourceEditor } from "./primitives/SourceEditor";
export type { SourceEditorProps } from "./primitives/SourceEditor";

export { SplitPane } from "./primitives/SplitPane";
export type { SplitPaneProps, SplitLayout } from "./primitives/SplitPane";

export { Segmented } from "./primitives/Segmented";
export type { SegmentedProps, SegmentedOption } from "./primitives/Segmented";

/// A modal card over the whole window, on a native `<dialog>`.
export { Sheet } from "./primitives/Sheet";
export type { SheetProps } from "./primitives/Sheet";

/// A disclosure tree with Finder's key map.
export { Tree, TreeRow } from "./primitives/Tree";
export type { TreeProps, TreeRowProps } from "./primitives/Tree";

export { Field, Input, Select, Switch } from "./primitives/Field";
export type {
  FieldProps,
  InputProps,
  SelectProps,
  SelectOption,
  SwitchProps,
} from "./primitives/Field";

export { Stack, Row, Spacer } from "./primitives/Layout";
export type { StackProps, RowProps, Gap, Align } from "./primitives/Layout";

export { Meter } from "./primitives/Meter";
export type { MeterProps } from "./primitives/Meter";

/// A status message over the window, from the app root or a sheet overlay.
export { Toast } from "./primitives/Toast";
export type { ToastProps, ToastTone } from "./primitives/Toast";
