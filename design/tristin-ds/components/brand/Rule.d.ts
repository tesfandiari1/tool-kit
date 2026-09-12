/** A 1px hairline divider in the ink tone. Blocks are separated by rules, never by gaps or shadows. */
export interface RuleProps {
  orientation?: 'horizontal' | 'vertical';
  /** ink on bone; bone on dark; slate for slate grounds (soft only) */
  tone?: 'ink' | 'slate' | 'bone';
  /** 28% alpha version for secondary divisions */
  soft?: boolean;
  style?: React.CSSProperties;
}
export function Rule(props: RuleProps): JSX.Element;
