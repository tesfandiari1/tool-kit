/** Text tabs in tracked caps along a 1px rule; the active tab thickens its own rule to ink. */
export interface TabsProps {
  items: Array<string | { value: string; label: string }>;
  value?: string;
  defaultValue?: string;
  onChange?: (value: string) => void;
  tone?: 'ink' | 'bone';
  style?: React.CSSProperties;
}
export function Tabs(props: TabsProps): JSX.Element;
