import { useEffect, useRef, useState } from "react";
import { commands } from "@/app/commands";
import { confirm, currentWindow, onDragDrop } from "@/platform/host";

/// The window itself: how files get in, how the chrome reacts to focus, and
/// what closing means. Three separate host subscriptions, each with its own
/// unsubscribe, grouped because they are all "the OS talking to us" and none
/// of them is about jobs.

/// Files arrive by drop anywhere in the window. Returns whether a drag is
/// currently over it, which the input well renders as an affordance.
export function useDragDrop(onDrop: (paths: string[]) => void) {
  const [dragging, setDragging] = useState(false);

  useEffect(() => {
    const un = onDragDrop((p) => {
      if (p.type === "over" || p.type === "enter") setDragging(true);
      else if (p.type === "drop") {
        setDragging(false);
        onDrop(p.paths);
      } else setDragging(false);
    });
    return () => void un.then((f) => f());
  }, [onDrop]);

  return dragging;
}

/// macOS dims an inactive window's accents; mirror it on `<body>` so the
/// stylesheet can follow with `body.inactive`.
export function useWindowFocusClass() {
  useEffect(() => {
    const win = currentWindow();
    void win.isFocused().then((f) => document.body.classList.toggle("inactive", !f));
    const un = win.onFocusChanged(({ payload }) =>
      document.body.classList.toggle("inactive", !payload),
    );
    return () => void un.then((f) => f());
  }, []);
}

/// This is a menu-bar app: closing the window hides it rather than quitting,
/// so the tray and ⌥⌘V keep working. Quit via the tray menu or ⌘Q. Closing
/// mid-run would abandon files already paid for upstream, so that asks first.
///
/// The handler is registered once and reads `running` / `activeCount` through
/// refs. Re-registering on every job event would be a subscription churn on a
/// 200-file run, and the closure would still be one render stale at the moment
/// it matters.
export function useCloseConfirm(running: boolean, activeCount: number) {
  const runningRef = useRef(running);
  const activeRef = useRef(activeCount);

  useEffect(() => {
    runningRef.current = running;
    activeRef.current = activeCount;
  }, [running, activeCount]);

  useEffect(() => {
    const win = currentWindow();
    const un = win.onCloseRequested(async (e) => {
      e.preventDefault();
      if (!runningRef.current) {
        await win.hide();
        return;
      }
      const active = activeRef.current;
      const keepGoing = await confirm(
        `${active} file${active > 1 ? "s are" : " is"} still processing. ` +
          "Keep the run going in the menu bar, or stop it and quit?",
        {
          title: "Tool-Kit is still working",
          kind: "warning",
          okLabel: "Keep working",
          cancelLabel: "Stop and quit",
        },
      );
      if (keepGoing) await win.hide();
      else {
        await commands.stopRun().catch(() => undefined);
        await commands.quitApp().catch(() => undefined);
      }
    });
    return () => void un.then((f) => f());
  }, []);
}
