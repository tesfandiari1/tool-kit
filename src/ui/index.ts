/// Tool-Kit UI — the design system's entire public surface.
///
/// Import from `@ui` and nothing deeper:
///
///     import { Button, Panel, Label } from "@ui";
///
/// This module has one hard rule, enforced by ESLint: nothing under `src/ui`
/// may import from `@/domains`, `@/app`, `@/platform`, or `@tauri-apps/*`. The
/// library knows about React and CSS, and nothing about Tool-Kit. That is what
/// keeps it lift-and-shift into `packages/ui` the day a second client needs it.
///
/// See UI.md for the design language and the rules for extending it.

import "./base.css";

export { cx } from "./cx";

export { Label, Display, Text, Mono } from "./primitives/Text";
export type {
  LabelProps,
  LabelTone,
  DisplayProps,
  DisplaySize,
  TextProps,
  TextSize,
  TextTone,
  MonoProps,
  MonoSize,
  MonoTone,
} from "./primitives/Text";

export { Button } from "./primitives/Button";
export type { ButtonProps, ButtonVariant, ButtonSize } from "./primitives/Button";

export { Panel, CellGrid, Cell, Well, Divider } from "./primitives/Surface";
export type {
  PanelProps,
  PanelTone,
  CellGridProps,
  CellProps,
  WellProps,
  DividerProps,
} from "./primitives/Surface";

export { Badge, Status, StatusDot } from "./primitives/Badge";
export type {
  BadgeProps,
  Tone,
  StatusProps,
  StatusTone,
  StatusDotProps,
  DotTone,
} from "./primitives/Badge";

export { Segmented } from "./primitives/Segmented";
export type { SegmentedProps, SegmentedOption } from "./primitives/Segmented";

export { Disclosure } from "./primitives/Disclosure";
export type { DisclosureProps } from "./primitives/Disclosure";

export { Field, Input, TextInput, Select, Switch } from "./primitives/Field";
export type {
  FieldProps,
  InputProps,
  TextInputProps,
  SelectProps,
  SelectOption,
  SwitchProps,
} from "./primitives/Field";

export { Stack, Row, Spacer } from "./primitives/Layout";
export type { StackProps, RowProps, Gap, Align, Justify } from "./primitives/Layout";

export { Meter } from "./primitives/Meter";
export type { MeterProps, MeterTone } from "./primitives/Meter";
