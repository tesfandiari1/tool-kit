import { Fragment } from "react";
import { pathCrumbs } from "./pathCrumbs";
import { cx } from "../cx";
import "./Path.css";

export interface PathProps {
  path: string;
  className?: string;
}

/// A file path as crumbs, with the last one the file.
///
/// The subhead face, because paths are identifiers. The trail is faint and the
/// file is not: you scan for the file and read the trail only when you need it.
export function Path({ path, className }: PathProps) {
  const crumbs = pathCrumbs(path);
  const last = crumbs.length - 1;

  return (
    <div className={cx("ui-path", className)} title={path}>
      {crumbs.map((crumb, i) => (
        <Fragment key={`${crumb}-${String(i)}`}>
          {i > 0 && crumbs[i - 1] !== "/" && (
            <span className="ui-path__sep" aria-hidden>
              /
            </span>
          )}
          <span className={cx("ui-path__crumb", i === last && "ui-path__crumb--leaf")}>
            {crumb}
          </span>
        </Fragment>
      ))}
    </div>
  );
}
