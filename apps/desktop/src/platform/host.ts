import { LogicalSize } from "@tauri-apps/api/dpi";
import { getCurrentWebview, type DragDropEvent } from "@tauri-apps/api/webview";
import { currentMonitor, getCurrentWindow } from "@tauri-apps/api/window";
import { confirm as confirmDialog, open as openDialog } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";

/// Host primitives (dialogs, window, drag-drop). Domain views should not import
/// `@tauri-apps/*` directly — swap this module later if a second host appears.

export async function pickFiles(): Promise<string[]> {
  const sel = await openDialog({ multiple: true });
  return normalizePaths(sel);
}

export async function pickFolders(): Promise<string[]> {
  const sel = await openDialog({ directory: true, multiple: true });
  return normalizePaths(sel);
}

export async function pickDirectory(): Promise<string | null> {
  const dir = await openDialog({ directory: true });
  return typeof dir === "string" ? dir : null;
}

export function confirm(message: string, options: Parameters<typeof confirmDialog>[1]) {
  return confirmDialog(message, options);
}

export function onDragDrop(handler: (event: DragDropEvent) => void) {
  return getCurrentWebview().onDragDropEvent((event) => {
    handler(event.payload);
  });
}

export function currentWindow() {
  return getCurrentWindow();
}

export interface WindowSize {
  width: number;
  height: number;
}

/// Window sizing is logical pixels on both sides of this boundary: the sizes in
/// `tauri.conf.json` and the persisted expanded size are logical, but
/// `innerSize()` answers in physical pixels, so on a retina display a window
/// reads back at twice the size it was just set to unless it is converted here.
export async function windowSize(): Promise<WindowSize> {
  const win = getCurrentWindow();
  const [size, scale] = await Promise.all([win.innerSize(), win.scaleFactor()]);
  const { width, height } = size.toLogical(scale);
  return { width, height };
}

/// Fires on every step of a live drag as well as on our own `resizeWindow`.
/// The size is deliberately not passed through: the event carries physical
/// pixels, and `windowSize` above is the one place that conversion lives.
export function onWindowResized(handler: () => void): Promise<() => void> {
  return getCurrentWindow().onResized(() => {
    handler();
  });
}

/// The window became key. `getCurrentWindow()`, never `getCurrentWebview()`:
/// a window focus event reaches only targets of kind Window and WebviewWindow,
/// so a webview-registered listener subscribes cleanly and then never fires,
/// with no error and no warning anywhere. The drag-drop subscription above uses
/// the webview, which is what makes that the natural wrong move here.
export function onWindowFocused(handler: () => void): Promise<() => void> {
  return getCurrentWindow().onFocusChanged(({ payload }) => {
    if (payload) handler();
  });
}

export async function resizeWindow(width: number, height: number): Promise<void> {
  await getCurrentWindow().setSize(new LogicalSize(width, height));
}

export async function setWindowMinSize(width: number, height: number): Promise<void> {
  await getCurrentWindow().setMinSize(new LogicalSize(width, height));
}

/// The launcher sizes itself to its content, so it stays fixed. A document
/// pane has to be growable.
export async function setWindowResizable(resizable: boolean): Promise<void> {
  await getCurrentWindow().setResizable(resizable);
}

export async function setWindowMaxSize(width: number, height: number): Promise<void> {
  await getCurrentWindow().setMaxSize(new LogicalSize(width, height));
}

/// macOS clamps `setSize` to the bounds in force at the time, so the launcher
/// phase has to drop the workspace ceiling before it can shrink back down.
export async function clearWindowMaxSize(): Promise<void> {
  await getCurrentWindow().setMaxSize(null);
}

/// The screen area left after the menu bar and the Dock, in logical pixels:
/// `workArea` answers in physical ones, and every caller compares this against
/// a logical window size. `currentMonitor()` also reports the display the
/// window is on, which `window.screen` does not.
export async function workArea(): Promise<WindowSize> {
  const monitor = await currentMonitor();
  // Null off a real host (the gallery runs in a plain browser tab), where the
  // screen object is the only answer available.
  if (!monitor) return { width: window.screen.availWidth, height: window.screen.availHeight };
  const { width, height } = monitor.workArea.size.toLogical(monitor.scaleFactor);
  return { width, height };
}

/// Page zoom scales the whole app, chrome included: in a Tauri window the page
/// is the app. The webview's own zoom hotkeys stay off so nothing competes.
export async function setWebviewZoom(factor: number): Promise<void> {
  await getCurrentWebview().setZoom(factor);
}

function normalizePaths(sel: string | string[] | null): string[] {
  if (Array.isArray(sel)) return sel;
  if (typeof sel === "string") return [sel];
  return [];
}

/// Open a http(s) link in the system browser. Markdown in the document pane
/// is full of links; letting the webview navigate would lose the open queue.
export async function openExternal(url: string): Promise<void> {
  await openUrl(url);
}

/// Copy to the clipboard, reporting success rather than throwing. Both callers
/// (a finished job row and an opened thread) want the same two outcomes and
/// the same two messages, so the wording stays with them and only the
/// capability lives here.
export async function copyToClipboard(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}
