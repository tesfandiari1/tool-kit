/** Ruled rectangle chip for topics and filters. Inverts to ink when selected. */
export interface TagProps {
  children: React.ReactNode;
  selected?: boolean;
  onClick?: () => void;
  onRemove?: () => void;
  tone?: 'ink' | 'bone';
}
export function Tag(props: TagProps): JSX.Element;
