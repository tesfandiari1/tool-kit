import { useEffect, useRef } from "react";
import { resizeWindow, windowSize, workArea } from "@/platform/host";
import { LAUNCHER, SCREEN_MARGIN } from "./geometry";

/// The launcher's height is its content's height.
///
/// The window is not a canvas the user drags to size (`resizable: false` in
/// tauri.conf.json). It is a container that fits what is inside it, so at any
/// moment there is exactly one correct height. Width belongs to the phase, not
/// to the content, and is never touched here.
///
/// **One rule: nothing the window sizes may size the window.** Measuring a
/// `flex: 1` column, a `100vh` frame, or `documentElement` oscillates or
/// collapses. Capping the shell at the ceiling latches, because a pinned box
/// reports the ceiling as its own height and stops resizing, so the observer
/// never fires again. So the compact shell is plain document flow and `#root`
/// is always exactly as tall as its content. Content taller than the screen
/// stops growing the window and scrolls the page instead, under a sticky title
/// bar.

/// Sub-pixel churn from font loading must not cost an IPC round trip.
const EPSILON = 2;

/// The tallest the launcher may grow, in logical pixels. `workArea()` rather
/// than `window.screen`: it reports the monitor the window is on, and in
/// logical pixels, which page zoom does not move.
async function ceiling(): Promise<number> {
  // No ceiling when the work area is unknown, rather than the minimum. Pinning
  // the launcher at its minimum clips content, letting it grow only costs a
  // scrollbar.
  const unknown = Number.MAX_SAFE_INTEGER;
  try {
    const { height } = await workArea();
    if (height <= 0) return unknown;
    return Math.max(LAUNCHER.minHeight, Math.round(height) - SCREEN_MARGIN);
  } catch {
    return unknown;
  }
}

export function useFitWindow(active: boolean, zoom: number): void {
  const busy = useRef(false);
  const again = useRef(false);
  const applied = useRef(0);

  useEffect(() => {
    const root = document.documentElement;
    const target = document.getElementById("root");
    if (!active) {
      root.classList.remove("fit-window", "is-capped", "is-scrolled");
      applied.current = 0;
      return;
    }

    root.classList.add("fit-window");

    let raf = 0;
    const fit = async () => {
      if (busy.current) {
        // Coalesce rather than drop, or a change that lands mid-resize is
        // never measured again.
        again.current = true;
        return;
      }
      busy.current = true;
      try {
        const content = target ? target.scrollHeight : 0;
        if (content <= 0) return;

        // Page zoom shrinks the CSS viewport, so `scrollHeight` is CSS pixels
        // while the window is set in logical ones. Convert once, here. The
        // minimum and the ceiling are both logical.
        const wanted = Math.ceil(content * zoom);

        const max = await ceiling();
        // This class sets the viewport's overflow and nothing else. No box
        // takes a height from it, so the measurement above stays honest.
        root.classList.toggle("is-capped", wanted > max);

        const height = Math.min(Math.max(wanted, LAUNCHER.minHeight), max);
        if (Math.abs(height - applied.current) < EPSILON) return;

        const { width } = await windowSize();
        await resizeWindow(width, height);
        // Only once the resize has landed. Recording the intent instead leaves
        // the hook believing in a height the window never took.
        applied.current = height;
      } catch {
        // The gallery and any other non-Tauri host have no window to resize.
      } finally {
        busy.current = false;
        if (again.current) {
          again.current = false;
          schedule();
        }
      }
    };

    const schedule = () => {
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(() => void fit());
    };

    // Content changes only. A `resize` listener here would react to this hook's
    // own resize, which is the same cycle from the other end.
    const ro = new ResizeObserver(schedule);
    if (target) ro.observe(target);
    schedule();

    // At the ceiling the page scrolls under the title bar, which then takes the
    // material a macOS toolbar shows over scrolled content. Scroll position is
    // read, never written, so this cannot feed the loop above.
    const onScroll = () => {
      root.classList.toggle("is-scrolled", window.scrollY > 0);
    };
    window.addEventListener("scroll", onScroll, { passive: true });
    onScroll();

    return () => {
      root.classList.remove("fit-window", "is-capped", "is-scrolled");
      ro.disconnect();
      window.removeEventListener("scroll", onScroll);
      cancelAnimationFrame(raf);
      applied.current = 0;
    };
    // Zoom changes the window the same content wants without changing the
    // content, so the ResizeObserver never fires. Re-running the effect does.
  }, [active, zoom]);
}
