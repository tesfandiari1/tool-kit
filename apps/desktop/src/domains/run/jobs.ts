import { FileTextIcon, WaveformIcon, type Icon } from "@phosphor-icons/react";
import type { JobId } from "@/app/types";

export interface JobDef {
  id: JobId;
  /// The job's name on the segmented control, and the Run button's verb.
  verb: string;
  desc: string;
  icon: Icon;
}

export const JOBS: JobDef[] = [
  { id: "convert", verb: "Convert", desc: "Documents to Markdown", icon: FileTextIcon },
  { id: "transcribe", verb: "Transcribe", desc: "Audio and video to text", icon: WaveformIcon },
];
