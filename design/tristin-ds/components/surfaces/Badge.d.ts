/** Status indicator: a 6px square swatch and tracked caps text, no pill, no fill. */
export interface BadgeProps {
  children: React.ReactNode;
  status?: 'neutral' | 'info' | 'success' | 'warning' | 'danger';
  tone?: 'ink' | 'bone';
}
export function Badge(props: BadgeProps): JSX.Element;
