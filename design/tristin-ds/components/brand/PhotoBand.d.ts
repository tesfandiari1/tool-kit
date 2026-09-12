/** Full-bleed dusk photograph under a flat dark wash, with bone type on top. The fourth colour. */
export interface PhotoBandProps {
  /** Image URL (use assets/photos/*) */
  src: string;
  /** normal .55, strong .72, none */
  wash?: 'normal' | 'strong' | 'none';
  /** min-height in px */
  height?: number;
  /** content placement */
  align?: 'center' | 'start' | 'end';
  children?: React.ReactNode;
  style?: React.CSSProperties;
}
export function PhotoBand(props: PhotoBandProps): JSX.Element;
