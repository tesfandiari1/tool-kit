import { useCallback, useEffect, useRef, useState } from "react";

/// How long a toast stays up. Long enough to read a provider error, short
/// enough that it is gone before the next one needs the slot.
const TOAST_MS = 3400;

/// The app's one channel for errors that never reach a job row: key saves,
/// reveal failures, clipboard. Owning the timer here is the point, because the
/// previous timeout has to be cleared both when a new message replaces it and
/// when the app unmounts.
export function useToast() {
  const [toast, setToast] = useState<string | null>(null);
  const timer = useRef<number | null>(null);

  const showToast = useCallback((msg: string) => {
    setToast(msg);
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
