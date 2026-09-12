import { useCallback, useEffect, useRef, useState } from "react";

/// How long a toast stays up. Long enough to read a provider error, short
/// enough that it is gone before the next one needs the slot.
const TOAST_MS = 3400;

export type ToastTone = "info" | "danger";

export interface ToastState {
  text: string;
  tone: ToastTone;
}

/// The app's one channel for errors that never reach a job row: key saves,
/// reveal failures, clipboard. Owning the timer here is the point, because the
/// previous timeout has to be cleared both when a new message replaces it and
/// when the app unmounts. The tone rides along, so a failure never reads as a
/// confirmation.
export function useToast() {
  const [toast, setToast] = useState<ToastState | null>(null);
  const timer = useRef<number | null>(null);

  const showToast = useCallback((msg: string, tone: ToastTone = "info") => {
    setToast({ text: msg, tone });
    if (timer.current) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setToast(null), TOAST_MS);
  }, []);

  useEffect(
    () => () => {
      if (timer.current) window.clearTimeout(timer.current);
    },
    [],
  );

  return { toast, showToast };
}
