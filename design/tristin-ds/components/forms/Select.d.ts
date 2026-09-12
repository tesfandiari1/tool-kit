/** Native select styled as a 44px rectangle with a typographic caret. */
export interface SelectProps {
  id?: string;
  value?: string;
  defaultValue?: string;
  onChange?: (e: React.ChangeEvent<HTMLSelectElement>) => void;
  options: Array<string | { value: string; label: string }>;
  placeholder?: string;
  disabled?: boolean;
  tone?: 'ink' | 'bone';
  style?: React.CSSProperties;
}
export function Select(props: SelectProps): JSX.Element;
