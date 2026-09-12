/** Ink rectangle with bone text, fades in on hover or focus. No arrow, no radius. */
export interface TooltipProps {
  content: React.ReactNode;
  children: React.ReactNode;
  side?: 'top' | 'bottom' | 'left' | 'right';
}
export function Tooltip(props: TooltipProps): JSX.Element;
