/** Ink bar with a 4px status edge on the left. Rectangular, no shadow, bottom-left of the viewport. */
export interface ToastProps {
  children: React.ReactNode;
  status?: 'neutral' | 'info' | 'success' | 'warning' | 'danger';
  /** Inline action, e.g. a ghost Button or link */
  action?: React.ReactNode;
  onDismiss?: () => void;
  /** ink (default) or bone panel */
  tone?: 'ink' | 'bone';
  style?: React.CSSProperties;
}
export function Toast(props: ToastProps): JSX.Element;
