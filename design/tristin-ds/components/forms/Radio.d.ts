/** Square radio (a square inside a square: buildings have corners). Use RadioGroup for a set. */
export interface RadioProps {
  name: string;
  value: string;
  label?: React.ReactNode;
  checked?: boolean;
  onChange?: (value: string, e: React.ChangeEvent) => void;
  disabled?: boolean;
  tone?: 'ink' | 'bone';
}
export function Radio(props: RadioProps): JSX.Element;
export interface RadioGroupProps {
  name: string;
  options: Array<string | { value: string; label: string }>;
  value?: string;
  onChange?: (value: string) => void;
  direction?: 'row' | 'column';
  tone?: 'ink' | 'bone';
}
export function RadioGroup(props: RadioGroupProps): JSX.Element;
