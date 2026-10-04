import { useCallback, useEffect, useRef, useState } from "react";
import { commands } from "@/app/commands";
import type { SpeechModel } from "@/app/types";

export interface SpeechModelFailure {
  message: string;
  /// What "Try again" repeats.
  retry: "check" | "download";
}

/// Lives in App, not in the sheet, so a download keeps its progress and its
/// result across a closed and reopened Settings.
export function useSpeechModel() {
  /// Undefined until the first check answers. Null on a build with no audio
  /// worker, which offers no Transcribe either.
  const [model, setModel] = useState<SpeechModel | null | undefined>(undefined);
  /// 0 to 1 while this app's download runs, null otherwise.
  const [progress, setProgress] = useState<number | null>(null);
  const [failure, setFailure] = useState<SpeechModelFailure | null>(null);
  const downloading = useRef(false);
  /// Only the newest check may land: two quick picks race otherwise.
  const latest = useRef(0);

  const refresh = useCallback(async (locale: string | null) => {
    if (downloading.current) return;
    const ticket = ++latest.current;
    try {
      const next = await commands.speechModel(locale);
      if (ticket !== latest.current) return;
      setModel(next);
      setFailure(null);
    } catch (e) {
      if (ticket === latest.current) setFailure({ message: String(e), retry: "check" });
    }
  }, []);

  const download = useCallback(async (locale: string | null) => {
    if (downloading.current) return;
    downloading.current = true;
    const ticket = ++latest.current;
    setFailure(null);
    setProgress(0);
    try {
      const next = await commands.downloadSpeechModel(locale);
      if (ticket === latest.current) setModel(next);
    } catch (e) {
      setFailure({ message: String(e), retry: "download" });
    } finally {
      downloading.current = false;
      setProgress(null);
    }
  }, []);

  useEffect(() => {
    // A late event must not bring the bar back after the download ended.
    const un = commands.onSpeechModelProgress((done) => {
      if (downloading.current) setProgress(done);
    });
    return () => void un.then((f) => f());
  }, []);

  return { model, progress, failure, refresh, download };
}
