import { useEffect } from "react";
import { setWebviewZoom } from "@/platform/host";
import { ZOOM } from "./geometry";

/// Cmd +/-/0 scale the whole app. The caller persists the factor.

/// Stepping picks a rung rather than adding, because `1 + 0.1 - 0.1` is
/// 0.9999999999999999. Rounded to two decimals, so a persisted factor matches.
const LADDER = buildLadder();

/// A factor through JSON lands a hair off its rung. Treat it as on.
const EPSILON = 1e-6;

function buildLadder(): number[] {
  const rungs: number[] = [];
  const steps = Math.round((ZOOM.max - ZOOM.min) / ZOOM.step);
  for (let i = 0; i <= steps; i++) {
    rungs.push(Math.round((ZOOM.min + i * ZOOM.step) * 100) / 100);
  }
  return rungs;
}

/// One rung, clamped. Direction 0 resets.
export function nextZoom(current: number, direction: -1 | 0 | 1): number {
  if (direction === 0) return ZOOM.default;
  if (direction === 1) {
    for (const rung of LADDER) {
      if (rung > current + EPSILON) return rung;
    }
    return LADDER[LADDER.length - 1];
  }
  for (let i = LADDER.length - 1; i >= 0; i--) {
    if (LADDER[i] < current - EPSILON) return LADDER[i];
  }
  return LADDER[0];
}

export function zoomLabel(factor: number): string {
  return `${Math.round(factor * 100)}%`;
}

export function useZoom(zoom: number, onChange: (next: number) => void): void {
  useEffect(() => {
    // For chrome measured against the traffic lights, which macOS draws in
    // logical pixels that page zoom never touches.
    document.documentElement.style.setProperty("--zoom", String(zoom));
    // The gallery runs in a browser tab with no webview to zoom.
    void setWebviewZoom(zoom).catch(() => undefined);
  }, [zoom]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      // Shift allowed, since Cmd++ and Cmd+_ are shifted keys. Text fields
      // are not exempt, as in every desktop app.
      if (!event.metaKey || event.ctrlKey || event.altKey) return;
      const direction = zoomDirection(event);
      if (direction === null) return;
      event.preventDefault();
      onChange(nextZoom(zoom, direction));
    };
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [zoom, onChange]);
}

/// The numpad's `key` depends on Num Lock, so it is matched by `code`.
function zoomDirection(event: KeyboardEvent): -1 | 0 | 1 | null {
  if (event.code === "NumpadAdd") return 1;
  if (event.code === "NumpadSubtract") return -1;
  if (event.code === "Numpad0") return 0;
  switch (event.key) {
    case "=":
    case "+":
      return 1;
    case "-":
    case "_":
      return -1;
    case "0":
      return 0;
    default:
      return null;
  }
}
