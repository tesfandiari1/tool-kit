/** 44px rectangular text field on a bone-lift ground with a 1px ink border. Focus turns the border deep slate. */
export interface InputProps {
  id?: string;
  value?: string;
  defaultValue?: string;
  onChange?: (e: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => void;
  placeholder?: string;
  type?: string;
  /** Renders a textarea */
  multiline?: boolean;
  rows?: number;
  disabled?: boolean;
  invalid?: boolean;
  /** bone for use on ink/photo grounds */
  tone?: 'ink' | 'bone';
  style?: React.CSSProperties;
}
export function Input(props: InputProps): JSX.Element;
