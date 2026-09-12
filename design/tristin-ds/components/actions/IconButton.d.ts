/** Square 44px (or 36px) button holding a single glyph or icon. Same variants as Button. */
export interface IconButtonProps {
  /** The glyph or icon element (Lucide SVG or a typographic character) */
  children: React.ReactNode;
  /** Accessible name, required */
  label: string;
  variant?: 'primary' | 'secondary' | 'inverse' | 'ghost';
  size?: 'md' | 'sm';
  disabled?: boolean;
  onClick?: (e: React.MouseEvent) => void;
  style?: React.CSSProperties;
}
export function IconButton(props: IconButtonProps): JSX.Element;
