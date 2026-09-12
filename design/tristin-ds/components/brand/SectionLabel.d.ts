/** Small tracked caps label in muted contrast, centred above a block (e.g. SELECTED WORK). */
export interface SectionLabelProps {
  children: React.ReactNode;
  /** ink on bone, slate on slate ground, bone on dark */
  tone?: 'ink' | 'slate' | 'bone';
  align?: 'center' | 'left';
  style?: React.CSSProperties;
}
export function SectionLabel(props: SectionLabelProps): JSX.Element;
