import { useEffect } from "react";
import { setWebviewZoom } from "@/platform/host";
import { ZOOM } from "./geometry";

/// Cmd +/-/0 scale the whole app: in a Tauri window the page is the app. The
/// hook applies a factor and reports the next one. Persisting it is the
/// caller's job.

/// The rungs, built once. Stepping picks a rung rather than adding to the
/// factor, because `1 + 0.1 - 0.1` is 0.9999999999999999 and arithmetic would
/// drift off the ladder. Each rung is rounded to two decimals so a persisted
/// factor compares equal to one.
const LADDER = buildLadder();

/// A factor that has been through JSON and back can land a hair off its rung.
/// Treat that as on it, so a step still moves exactly one rung.
const EPSILON = 1e-6;

function buildLadder(): number[] {
  const rungs: number[] = [];
  const steps = Math.round((ZOOM.max - ZOOM.min) / ZOOM.step);
  for (let i = 0; i <= steps; i++) {
    rungs.push(Math.round((ZOOM.min + i * ZOOM.step) * 100) / 100);
  }
  return rungs;
}

/// One rung along the ladder, clamped at both ends. Direction 0 resets.
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

/// The factor as the toast says it: percent, no decimals.
export function zoomLabel(factor: number): string {
  return `${Math.round(factor * 100)}%`;
}

export function useZoom(zoom: number, onChange: (next: number) => void): void {
  useEffect(() => {
    // Published for chrome measured against the traffic lights. macOS draws
    // those in logical pixels, which page zoom never touches, so App.css
    // divides those lengths by `--zoom` to convert.
    document.documentElement.style.setProperty("--zoom", String(zoom));
    // The gallery runs in a plain browser tab with no webview to zoom, so
    // swallow the failure.
    void setWebviewZoom(zoom).catch(() => undefined);
  }, [zoom]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      // Meta, optionally with Shift, since Cmd++ and Cmd+_ are shifted keys.
      // Any other modifier means a different command. Text fields are not
      // exempt: Cmd+- zooms while you type, the same as every desktop app.
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

/// The numpad keys report a `key` that depends on Num Lock, so they are matched
/// by `code` while the main row is matched by `key`.
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
