/** A rectangle bounded by 1px rules on a flat ground. No fill change, no radius, no shadow. Optional washed image on top. */
export interface CardProps {
  /** Small tracked caps above the title */
  label?: string;
  title?: React.ReactNode;
  children?: React.ReactNode;
  /** Image URL; rendered 16:10 under the photo wash */
  image?: string;
  /** Footer meta line */
  meta?: React.ReactNode;
  /** Makes the card a link with hover treatment */
  href?: string;
  tone?: 'bone' | 'ink' | 'slate';
  padding?: number;
  style?: React.CSSProperties;
}
export function Card(props: CardProps): JSX.Element;
