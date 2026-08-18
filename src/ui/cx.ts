/// Join class names, keeping only non-empty strings. The whole of the system's
/// class-composition needs, with zero dependencies.
///
/// Takes `unknown` rather than a narrow union so that the ordinary
/// `cond && "class"` idiom works when `cond` is a `ReactNode` or a number,
/// where `&&` can yield `0`, `0n`, or `""` rather than `false`.
export function cx(...parts: unknown[]): string {
  return parts.filter((p): p is string => typeof p === "string" && p !== "").join(" ");
}
