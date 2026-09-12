/** 18px square with a 1px ink border; fills ink when checked. No radius. */
export interface CheckboxProps {
  id?: string;
  label?: React.ReactNode;
  checked?: boolean;
  defaultChecked?: boolean;
  onChange?: (checked: boolean, e: React.ChangeEvent) => void;
  disabled?: boolean;
  tone?: 'ink' | 'bone';
}
export function Checkbox(props: CheckboxProps): JSX.Element;
