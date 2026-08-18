/// One opened result. The live run and history both produce this so the
/// viewer does not care which door you came through.
export interface ThreadDoc {
  title: string;
  subtitle: string | null;
  text: string | null;
  revealPath: string | null;
}
