/** Rectangular button in Red Hat Display tracked caps. Ink fill, ink outline, bone outline (for dark grounds), or ghost text. */
export interface ButtonProps {
  children: React.ReactNode;
  /** primary: ink fill. secondary: ink outline. inverse: bone outline for dark/slate grounds. ghost: text only */
  variant?: 'primary' | 'secondary' | 'inverse' | 'ghost';
  /** md 44px, sm 36px */
  size?: 'md' | 'sm';
  disabled?: boolean;
  /** Renders an anchor when set */
  href?: string;
  onClick?: (e: React.MouseEvent) => void;
  type?: 'button' | 'submit' | 'reset';
  style?: React.CSSProperties;
}
export function Button(props: ButtonProps): JSX.Element;
