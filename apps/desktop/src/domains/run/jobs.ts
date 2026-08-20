import { FileTextIcon, WaveformIcon, type Icon } from "@phosphor-icons/react";
import type { JobId, SecretId } from "@/app/types";

export interface JobDef {
  id: JobId;
  label: string;
  verb: string;
  desc: string;
  secret: SecretId;
  service: string;
  icon: Icon;
}

export const JOBS: JobDef[] = [
  { id: "convert", label: "Convert", verb: "Convert", desc: "Documents to Markdown", secret: "datalab", service: "Datalab", icon: FileTextIcon },
  { id: "transcribe", label: "Transcribe", verb: "Transcribe", desc: "Audio and video to text", secret: "revai", service: "Rev.ai", icon: WaveformIcon },
];
