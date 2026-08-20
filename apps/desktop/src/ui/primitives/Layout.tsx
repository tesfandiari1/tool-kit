import type { ElementType, HTMLAttributes, ReactNode } from "react";
import { cx } from "../cx";
import "./Layout.css";

/// Gap steps map to the 4px scale. There is no arbitrary spacing value.
export type Gap = 1 | 2 | 3 | 4 | 5 | 6;
export type Align = "start" | "center" | "end" | "stretch" | "baseline";
export type Justify = "start" | "center" | "end" | "between";

interface LayoutBase extends HTMLAttributes<HTMLElement> {
  as?: ElementType;
  gap?: Gap;
  align?: Align;
  justify?: Justify;
  children?: ReactNode;
}

export type StackProps = LayoutBase;

export function Stack({ as: As = "div", gap = 3, align, justify, className, ...rest }: StackProps) {
  return (
    <As
      className={cx(
        "ui-stack",
        `ui-gap-${String(gap)}`,
        align && `ui-align-${align}`,
        justify && `ui-justify-${justify}`,
        className,
      )}
      {...rest}
    />
  );
}

export interface RowProps extends LayoutBase {
  wrap?: boolean;
}

export function Row({
  as: As = "div",
  gap = 2,
  align = "center",
  justify,
  wrap = false,
  className,
  ...rest
}: RowProps) {
  return (
    <As
      className={cx(
        "ui-row",
        `ui-gap-${String(gap)}`,
        `ui-align-${align}`,
        justify && `ui-justify-${justify}`,
        wrap && "ui-row--wrap",
        className,
      )}
      {...rest}
    />
  );
}

/// Pushes everything after it to the far edge of a Row.
export function Spacer() {
  return <span className="ui-spacer" />;
}
