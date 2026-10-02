import { useEffect, useRef, useState } from "react";
import { commands } from "@/app/commands";
import { confirm, currentWindow, onDragDrop } from "@/platform/host";

/// Three host subscriptions: drop, focus and close.

/// Reports whether a drag is over the window, which the well renders.
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

/// Mirrored on `<body>`, so the stylesheet follows with `body.inactive`.
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

/// Closing hides the window rather than quit, so the tray keeps working, and
/// it asks first mid-run because a file already sent is paid for. The handler
/// registers once and reads the counts through refs, or a 200-file run churns
/// the subscription.
export function useCloseConfirm(running: boolean, activeCount: number, dirtyCount: number) {
  const runningRef = useRef(running);
  const activeRef = useRef(activeCount);
  const dirtyRef = useRef(dirtyCount);

  useEffect(() => {
    runningRef.current = running;
    activeRef.current = activeCount;
    dirtyRef.current = dirtyCount;
  }, [running, activeCount, dirtyCount]);

  useEffect(() => {
    const win = currentWindow();
    const un = win.onCloseRequested(async (e) => {
      e.preventDefault();
      // Hiding keeps an unsaved edit alive, and the quit below does not. The
      // case that matters is a refused write, which waiting does not fix.
      const dirty = dirtyRef.current;
      if (dirty > 0) {
        const keep = await confirm(
          `${String(dirty)} document${dirty > 1 ? "s have" : " has"} changes that are not on disk. ` +
            "Leave the window open to finish saving, or hide it and keep them in memory?",
          {
            title: dirty > 1 ? "Unsaved documents" : "Unsaved document",
            kind: "warning",
            okLabel: "Keep open",
            cancelLabel: "Hide anyway",
          },
        );
        if (keep) return;
      }
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
