/** Modal panel: bone-lift rectangle with a 1px ink border over a dark wash. No radius, no shadow. */
export interface DialogProps {
  open?: boolean;
  onClose?: () => void;
  /** Small tracked caps above the title */
  label?: string;
  title?: React.ReactNode;
  children?: React.ReactNode;
  /** Footer buttons */
  actions?: React.ReactNode;
  width?: number;
}
export function Dialog(props: DialogProps): JSX.Element;
