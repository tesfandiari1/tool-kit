/// Pane widths as percentages, keyed by pane id: `{ start: 33, end: 67 }`.
export type SplitLayout = Record<string, number>;

/// The layout to hand the group. Its own module, or fast refresh breaks.
/// Clamping `start` and deriving `end` repair a pair the group cannot take.
export function paneLayout(layout: SplitLayout | undefined, defaultStart: number): SplitLayout {
  const start = Math.min(100, Math.max(0, layout?.start ?? defaultStart));
  return { start, end: 100 - start };
}
