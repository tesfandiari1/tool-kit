import { useEffect, useRef } from "react";
import { resizeWindow, windowSize } from "@/platform/host";

/// The launcher's height is its content's height.
///
/// The window is not a canvas the user drags to size (`resizable: false` in
/// tauri.conf.json). It is a container that fits what is inside it, the way
/// System Settings resizes itself when you switch panes. So at any moment there
/// is exactly one correct height, and keeping the window at it is this hook's
/// whole job. Width belongs to the phase, not to the content, and is never
/// touched here.
///
/// Two rules keep that stable, and both were learned the hard way:
///
/// 1. **Measure something the window cannot stretch.** `#root` is in normal
///    flow with `height: auto`, so its `scrollHeight` is intrinsic: the same
///    number whatever the window is doing. Measuring the viewport instead (a
///    `flex: 1` column, a `100vh` frame, `documentElement.scrollHeight`) makes
///    the measurement depend on the window that the measurement is about to
///    set. A circular dependency like that either oscillates or collapses.
/// 2. **Never listen for window resizes.** Reacting to the resize this hook
///    just performed closes the same loop from the other end.
///
/// Content taller than the screen stops growing the window and scrolls inside
/// it instead. Nothing is ever clipped: the window either fits the content or
/// hands it a scroller, and `.is-capped` is how the stylesheet is told which.
const MIN_HEIGHT = 460;
/// Breathing room so a full-height launcher never sits flush against the work
/// area. `availHeight` has already taken the menu bar and Dock off.
const SCREEN_MARGIN = 48;
/// Sub-pixel churn from font loading must not cost an IPC round trip.
const EPSILON = 2;

function ceiling(): number {
  const avail = Math.round(window.screen.availHeight);
  // A zero here means a host that does not report a screen. Falling back to
  // "no ceiling" keeps the launcher growing rather than pinning it at the
  // minimum, which would clip on the one host that cannot tell us otherwise.
  return avail > 0 ? Math.max(MIN_HEIGHT, avail - SCREEN_MARGIN) : Number.MAX_SAFE_INTEGER;
}

export function useFitWindow(active: boolean) {
  const busy = useRef(false);
  const again = useRef(false);
  const applied = useRef(0);

  useEffect(() => {
    const root = document.documentElement;
    const target = document.getElementById("root");
    if (!active) {
      root.classList.remove("fit-window", "is-capped");
      root.style.removeProperty("--shell-max");
      applied.current = 0;
      return;
    }

    root.classList.add("fit-window");

    let raf = 0;
    const fit = async () => {
      if (busy.current) {
        // Coalesce rather than drop. The old guard returned outright, so a
        // change that landed mid-resize was never measured again and the
        // window kept a height its content had already outgrown.
        again.current = true;
        return;
      }
      busy.current = true;
      try {
        const max = ceiling();
        root.style.setProperty("--shell-max", `${max}px`);

        const content = target ? target.scrollHeight : 0;
        if (content <= 0) return;

        const capped = content > max;
        root.classList.toggle("is-capped", capped);

        const height = Math.min(Math.max(Math.ceil(content), MIN_HEIGHT), max);
        if (Math.abs(height - applied.current) < EPSILON) return;

        const { width } = await windowSize();
        await resizeWindow(width, height);
        // Only after the resize lands. Recording the intent instead leaves the
        // hook believing in a height the window never took.
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

    // Content changes only. A `resize` listener here would react to this
    // hook's own resize; see rule 2 above.
    const ro = new ResizeObserver(schedule);
    if (target) ro.observe(target);
    schedule();

    return () => {
      root.classList.remove("fit-window", "is-capped");
      root.style.removeProperty("--shell-max");
      ro.disconnect();
      cancelAnimationFrame(raf);
      applied.current = 0;
    };
  }, [active]);
}
