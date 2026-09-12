/** Rectangular toggle, 40x22, square thumb. Fills ink when on. */
export interface SwitchProps {
  id?: string;
  label?: React.ReactNode;
  checked?: boolean;
  defaultChecked?: boolean;
  onChange?: (checked: boolean) => void;
  disabled?: boolean;
  tone?: 'ink' | 'bone';
}
export function Switch(props: SwitchProps): JSX.Element;
