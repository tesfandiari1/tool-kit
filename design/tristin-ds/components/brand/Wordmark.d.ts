/** The text logo: TRISTIN ESFANDIARI in TRJN DaVinci tracked caps, with optional role line. There is no graphic mark. */
export interface WordmarkProps {
  /** sm 20px, md 32px, lg 56px */
  size?: 'sm' | 'md' | 'lg';
  /** Role line under the name, e.g. "Business Systems & Transformation Consultant" */
  tagline?: string;
  /** ink on light grounds, bone on dark/photo */
  tone?: 'ink' | 'bone';
  align?: 'center' | 'left';
  /** Element to render, e.g. 'h1' */
  as?: string;
}
export function Wordmark(props: WordmarkProps): JSX.Element;
