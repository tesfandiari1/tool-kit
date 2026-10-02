/// Pane widths as percentages, keyed by pane id: `{ start: 33, end: 67 }`.
export type SplitLayout = Record<string, number>;

/// The layout to hand the group. Its own module, or fast refresh breaks.
/// Deriving `end` repairs a pair that no longer sums to 100.
export function paneLayout(layout: SplitLayout | undefined, defaultStart: number): SplitLayout {
  const start = layout?.start ?? defaultStart;
  return { start, end: 100 - start };
}
