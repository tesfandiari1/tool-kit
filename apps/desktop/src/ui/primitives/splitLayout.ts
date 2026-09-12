/// Pane widths as percentages, keyed by pane id: `{ start: 33, end: 67 }`.
export type SplitLayout = Record<string, number>;

/// The layout to hand the group, built in panel order. Its own module, or fast
/// refresh breaks.
///
/// `react-resizable-panels` reads `defaultLayout` by panel id but re-keys
/// `setLayout` positionally, so key order decides which pane gets which width.
/// A layout from a Rust `BTreeMap` arrives `{ end, start }` and inverts the
/// split. Deriving `end` also repairs a pair that no longer sums to 100.
export function paneLayout(layout: SplitLayout | undefined, defaultStart: number): SplitLayout {
  const start = layout?.start ?? defaultStart;
  return { start, end: 100 - start };
}
