/** Tracked small-caps label above a form control, with optional right-aligned hint. */
export interface FieldLabelProps {
  children: React.ReactNode;
  htmlFor?: string;
  /** e.g. "Optional" */
  hint?: string;
}
export function FieldLabel(props: FieldLabelProps): JSX.Element;
