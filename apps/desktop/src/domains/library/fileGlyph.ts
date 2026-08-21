import {
  FileAudioIcon,
  FileCodeIcon,
  FileCsvIcon,
  FileDocIcon,
  FileHtmlIcon,
  FileIcon,
  FileImageIcon,
  FileMdIcon,
  FilePdfIcon,
  FilePptIcon,
  FileTextIcon,
  FileVideoIcon,
  FileXlsIcon,
  FileZipIcon,
  FolderIcon,
  type Icon,
} from "@phosphor-icons/react";

/// Extension to glyph.
///
/// Its own module because a helper exported beside a component fails
/// `react-refresh/only-export-components`, the same reason `pathCrumbs.ts` and
/// `splitLayout.ts` are separate files.
///
/// Monochrome throughout. Every glyph inherits the row's ink, so the tree
/// spends no colour on a file type: colour is signal here, and "this is a PDF"
/// is not one (UI.md rule 1).
const BY_EXT: Record<string, Icon> = {
  pdf: FilePdfIcon,
  md: FileMdIcon,
  markdown: FileMdIcon,
  txt: FileTextIcon,
  rtf: FileTextIcon,
  doc: FileDocIcon,
  docx: FileDocIcon,
  odt: FileDocIcon,
  xls: FileXlsIcon,
  xlsx: FileXlsIcon,
  csv: FileCsvIcon,
  ppt: FilePptIcon,
  pptx: FilePptIcon,
  html: FileHtmlIcon,
  htm: FileHtmlIcon,
  json: FileCodeIcon,
  xml: FileCodeIcon,
  yaml: FileCodeIcon,
  yml: FileCodeIcon,
  png: FileImageIcon,
  jpg: FileImageIcon,
  jpeg: FileImageIcon,
  gif: FileImageIcon,
  webp: FileImageIcon,
  tiff: FileImageIcon,
  tif: FileImageIcon,
  heic: FileImageIcon,
  bmp: FileImageIcon,
  mp3: FileAudioIcon,
  wav: FileAudioIcon,
  m4a: FileAudioIcon,
  aac: FileAudioIcon,
  flac: FileAudioIcon,
  ogg: FileAudioIcon,
  mp4: FileVideoIcon,
  mov: FileVideoIcon,
  m4v: FileVideoIcon,
  webm: FileVideoIcon,
  mkv: FileVideoIcon,
  zip: FileZipIcon,
};

/// The glyph a tree row wears. `ext` is the host's: lowercase and without the
/// dot, so nothing here parses a file name.
export function fileGlyph(ext: string, isDir: boolean): Icon {
  if (isDir) return FolderIcon;
  return BY_EXT[ext] ?? FileIcon;
}
