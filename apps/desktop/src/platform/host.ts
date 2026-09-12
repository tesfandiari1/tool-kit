import { LogicalPosition, LogicalSize } from "@tauri-apps/api/dpi";
import { getCurrentWebview, type DragDropEvent } from "@tauri-apps/api/webview";
import { currentMonitor, getCurrentWindow } from "@tauri-apps/api/window";
import { confirm as confirmDialog, open as openDialog } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";

/// Host primitives. No domain view imports `@tauri-apps/*` directly.

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

export interface WorkArea extends WindowSize {
  x: number;
  y: number;
}

/// `innerSize()` answers in physical pixels, and everything else here is
/// logical: unconverted, a retina window reads back at twice its size.
export async function windowSize(): Promise<WindowSize> {
  const win = getCurrentWindow();
  const [size, scale] = await Promise.all([win.innerSize(), win.scaleFactor()]);
  const { width, height } = size.toLogical(scale);
  return { width, height };
}

/// Fires on every step of a drag, and on our own `resizeWindow`. The size is
/// not passed through: `windowSize` owns the conversion.
export function onWindowResized(handler: () => void): Promise<() => void> {
  return getCurrentWindow().onResized(() => {
    handler();
  });
}

/// `getCurrentWindow()`, never `getCurrentWebview()`: a focus event reaches
/// only Window targets, so a webview listener subscribes and never fires.
export function onWindowFocused(handler: () => void): Promise<() => void> {
  return getCurrentWindow().onFocusChanged(({ payload }) => {
    if (payload) handler();
  });
}

export async function resizeWindow(width: number, height: number): Promise<void> {
  await getCurrentWindow().setSize(new LogicalSize(width, height));
}

/// Centre on the size we asked for: macOS applies `setSize` asynchronously, so
/// the host's own `center()` measures the old frame.
export async function centerWindow(width: number, height: number): Promise<void> {
  const area = await workArea();
  const x = area.x + (area.width - width) / 2;
  const y = area.y + (area.height - height) / 2;
  await getCurrentWindow().setPosition(new LogicalPosition(x, y));
}

/// The one place the hidden window becomes visible.
export async function showWindow(): Promise<void> {
  await getCurrentWindow().show();
}

export async function setWindowMinSize(width: number, height: number): Promise<void> {
  await getCurrentWindow().setMinSize(new LogicalSize(width, height));
}

/// The window opens fixed and unlocks once, when the workspace arrives.
export async function setWindowResizable(resizable: boolean): Promise<void> {
  await getCurrentWindow().setResizable(resizable);
}

export async function setWindowMaxSize(width: number, height: number): Promise<void> {
  await getCurrentWindow().setMaxSize(new LogicalSize(width, height));
}

/// What is left after the menu bar and the Dock, in logical pixels, from the
/// display the window is on. The origin comes with it: on a second monitor it
/// is not zero.
export async function workArea(): Promise<WorkArea> {
  const monitor = await currentMonitor();
  // Null off a real host, such as the gallery in a browser tab.
  if (!monitor) {
    const screen = window.screen as Screen & { availLeft?: number; availTop?: number };
    return {
      width: screen.availWidth,
      height: screen.availHeight,
      x: screen.availLeft ?? 0,
      y: screen.availTop ?? 0,
    };
  }
  const { width, height } = monitor.workArea.size.toLogical(monitor.scaleFactor);
  const { x, y } = monitor.workArea.position.toLogical(monitor.scaleFactor);
  return { width, height, x, y };
}

/// Page zoom scales the whole app: in a Tauri window the page is the app.
export async function setWebviewZoom(factor: number): Promise<void> {
  await getCurrentWebview().setZoom(factor);
}

function normalizePaths(sel: string | string[] | null): string[] {
  if (Array.isArray(sel)) return sel;
  if (typeof sel === "string") return [sel];
  return [];
}

/// The system browser: navigating the webview loses every open document.
export async function openExternal(url: string): Promise<void> {
  await openUrl(url);
}

/// Reports success rather than throwing. The wording stays with the caller.
export async function copyToClipboard(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}
