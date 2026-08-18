import { LogicalSize } from "@tauri-apps/api/dpi";
import { getCurrentWebview, type DragDropEvent } from "@tauri-apps/api/webview";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { confirm as confirmDialog, open as openDialog } from "@tauri-apps/plugin-dialog";

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

/// Window sizing is logical pixels on both sides of this boundary: the sizes in
/// `tauri.conf.json` and the persisted expanded size are logical, but
/// `innerSize()` answers in physical pixels, so on a retina display a window
/// reads back at twice the size it was just set to unless it is converted here.
export async function windowSize(): Promise<{ width: number; height: number }> {
  const win = getCurrentWindow();
  const [size, scale] = await Promise.all([win.innerSize(), win.scaleFactor()]);
  const { width, height } = size.toLogical(scale);
  return { width, height };
}

export async function resizeWindow(width: number, height: number): Promise<void> {
  await getCurrentWindow().setSize(new LogicalSize(width, height));
}

export async function setWindowMinSize(width: number, height: number): Promise<void> {
  await getCurrentWindow().setMinSize(new LogicalSize(width, height));
}

function normalizePaths(sel: string | string[] | null): string[] {
  if (Array.isArray(sel)) return sel;
  if (typeof sel === "string") return [sel];
  return [];
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
