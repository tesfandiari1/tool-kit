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
