/// Pane widths as percentages, keyed by pane id: `{ start: 33, end: 67 }`.
export type SplitLayout = Record<string, number>;

/// The layout to hand the group, built key by key in panel order.
///
/// Its own module because a file that exports a component cannot also export
/// this without breaking fast refresh.
///
/// `react-resizable-panels` has two layout paths that disagree about what a key
/// means. `defaultLayout` is read by panel id, but `setLayout` validates through
/// `Object.values(layout)` and re-keys the result positionally, so key order
/// decides which pane gets which width. A saved layout arrives from a Rust
/// `BTreeMap`, which serializes alphabetically as `{ end, start }`, so passing
/// it through opens the split inverted. Deriving `end` from `start` also
/// repairs a pair that no longer sums to 100, which the library throws on.
export function paneLayout(layout: SplitLayout | undefined, defaultStart: number): SplitLayout {
  const start = layout?.start ?? defaultStart;
  return { start, end: 100 - start };
}
