import { useEffect, useState } from "react";
import type { ReactNode } from "react";
import {
  Badge,
  Button,
  Display,
  Divider,
  Field,
  Input,
  Label,
  Meter,
  Meta,
  Panel,
  Path,
  Row,
  Segmented,
  Select,
  Sheet,
  SourceEditor,
  SplitPane,
  Spacer,
  Stack,
  StatusDot,
  Switch,
  Tabs,
  Text,
  Toast,
  Tree,
  TreeRow,
  Well,
} from "../index";
import type { SplitLayout } from "../index";
import "./Gallery.css";

/// Every primitive in every state. A component without a specimen here cannot
/// be reviewed, so add both in one commit. Open with `?gallery` in dev.

/// Real conversion output: the gutter has to be checked against wrapped lines.
const SAMPLE = `# Strategic Partner Agreement

**Parties.** Schild Technologies, a Delaware corporation, and the Partner identified in Exhibit A.

## 4.7 Vesting

Shares vest monthly over 48 months, with a 12-month cliff. **No acceleration on change of control.**

> Repurchase of vested shares is at fair market value as determined by the Company.

| Trigger             | Price             | Window  |
|---------------------|-------------------|---------|
| Voluntary departure | Fair market value | 90 days |

Cause is not defined. See \`Exhibit C\` for the notice procedure.
`;

const THEME_KEY = "toolkit.gallery.theme";
type Theme = "system" | "bone" | "ink";

export function Gallery() {
  const [tab, setTab] = useState("agreement");
  const [closed, setClosed] = useState<string[]>([]);
  const [source, setSource] = useState(SAMPLE);

  const [theme, setTheme] = useState<Theme>(
    () => {
      // A value from an older gallery (graphite, paper) must not leave every
      // option unselected.
      const stored = localStorage.getItem(THEME_KEY);
      return stored === "bone" || stored === "ink" ? stored : "system";
    },
  );
  // Interactive specimens: a segmented control and a disclosure are reviewable
  // only if you can operate them.
  const [job, setJob] = useState<"convert" | "transcribe">("convert");

  useEffect(() => {
    const root = document.documentElement;
    if (theme === "system") root.removeAttribute("data-theme");
    else root.setAttribute("data-theme", theme);
    localStorage.setItem(THEME_KEY, theme);
    return () => {
      root.removeAttribute("data-theme");
    };
  }, [theme]);

  return (
    <div className="gal">
      <div className="gal__inner">
        <div className="gal__bar">
          <Display size="lg">Tool-Kit UI</Display>
          <Badge>v0.1</Badge>
          <Spacer />
          <Label>Theme</Label>
          {/* The theme switch is also the Segmented specimen at `sm`. */}
          <Segmented
            label="Theme"
            size="sm"
            value={theme}
            onChange={setTheme}
            options={[
              { value: "system" as const, label: "System" },
              { value: "bone" as const, label: "Bone" },
              { value: "ink" as const, label: "Ink" },
            ]}
          />
        </div>

        <Section
          title="Colour"
          note="The role layer, and nothing else. Warning is live, success passed, danger failed."
        >
          <div className="gal__swatches">
            {(
              [
                ["--surface-page", "surface page"],
                ["--surface-lift", "surface lift"],
                ["--surface-sunk", "surface sunk"],
                ["--rule", "rule"],
                ["--rule-soft", "rule soft"],
                ["--text-body", "text body"],
                ["--text-soft", "text soft"],
                ["--text-mute", "text mute"],
                ["--text-ghost", "text ghost"],
                ["--link-hover", "link hover"],
                ["--focus-ring", "focus ring"],
                ["--selection", "selection"],
                ["--toast-bg", "toast bg"],
                ["--status-info", "status info"],
                ["--status-success", "status success"],
                ["--status-warning", "status warning"],
                ["--status-danger", "status danger"],
              ] as const
            ).map(([token, name]) => (
              <figure key={token} className="gal__swatch">
                <div className="gal__chip" style={{ background: `var(${token})` }} />
                <figcaption>
                  <Label>{name}</Label>
                </figcaption>
              </figure>
            ))}
          </div>
        </Section>

        <Section title="Typography" note="Three faces, three jobs, no overlap.">
          <div className="gal__specimen">
            <Stack gap={4}>
              <Display size="3xl">Less time configuring</Display>
              <Display size="2xl">Convert 24 files</Display>
              <Display size="xl">Run history</Display>
              <Divider />
              <Label>Label, the signature</Label>
              <Label tone="strong">Label strong</Label>
              <Divider />
              <Text size="lg">Large body copy for a lead paragraph.</Text>
              <Text>
                Base body copy. Drop files and folders, then run. The job is picked from what you
                dropped and the output folder sits alongside the input.
              </Text>
              <Text size="sm" tone="muted">
                Small muted copy, for hints beneath a control.
              </Text>
              <Text size="xs" tone="ghost">
                Ghost copy, holding a slot until it carries meaning.
              </Text>
              <Divider />
              <Meta>~/Deals/Acme/2024-Q3_board-deck.pdf</Meta>
              {/* Meta is the only face with tabular figures, so these two rows
                  measure the same width and a running timer never jitters. */}
              <Meta tone="ink">00:04:18</Meta>
              {/* Minus signs, not hyphens: the face gives U+2212 a digit's width. */}
              <Meta tone="ghost">−−:−−:−−</Meta>
            </Stack>
          </div>
        </Section>

        <Section title="Button" note="Every control wears the same tracked caps label.">
          <div className="gal__specimen">
            <Stack gap={4}>
              <Row gap={2} wrap>
                <Button variant="primary">Convert 3 files</Button>
                <Button variant="secondary">Stop</Button>
                <Button variant="ghost">Clear</Button>
              </Row>
              <Row gap={2} wrap>
                <Button variant="primary" size="sm">
                  Small
                </Button>
                <Button variant="secondary" size="sm">
                  Small secondary
                </Button>
                <Button variant="primary" disabled>
                  Disabled
                </Button>
                <Button variant="ghost">Clear history</Button>
              </Row>
              {/* What the run screen ships: the lg actuator, its busy state,
                  and icon-only chrome. */}
              <Row gap={2} align="stretch" wrap>
                <Button variant="primary" size="lg" icon="▶">
                  Convert 3 files
                </Button>
                <Button variant="primary" size="lg" busy icon="◌">
                  Working… 3 of 6
                </Button>
                <Button variant="secondary" size="lg" icon="■">
                  Stop
                </Button>
              </Row>
              <Row gap={2} wrap>
                <Button variant="ghost" iconOnly icon="⚙" aria-label="Settings" />
                <Button variant="ghost" size="sm" iconOnly icon="◉" aria-label="Preview" />
                <Button variant="secondary" iconOnly icon="⧉" aria-label="Copy" />
              </Row>
              <Button variant="primary" block>
                Block
              </Button>
              {/* Inverse only ever sits on a dark band, so the specimen brings
                  its own. */}
              <div className="gal__dark">
                <Row gap={2} wrap>
                  <Button variant="inverse">Choose folder</Button>
                  <Button variant="inverse" size="sm">
                    Small
                  </Button>
                  <Button variant="inverse" iconOnly icon="⧉" aria-label="Copy" />
                </Row>
              </div>
            </Stack>
          </div>
        </Section>

        <Section title="Status" note="Slots hold their size at every state, so lists never reflow.">
          <div className="gal__specimen">
            <Stack gap={4}>
              <Row gap={4} wrap>
                {(["idle", "queued", "live", "pass", "fault"] as const).map((t) => (
                  <Row key={t} gap={2}>
                    <StatusDot tone={t} label={t} />
                    <Label>{t}</Label>
                  </Row>
                ))}
              </Row>
              <Divider />
              <Row gap={2} wrap>
                <Badge>Universal-3.5 Pro</Badge>
                <Badge tone="info">Queued</Badge>
                <Badge tone="success">Done</Badge>
                <Badge tone="warning">Retrying</Badge>
                <Badge tone="danger">Failed</Badge>
                <Badge count>24</Badge>
                {/* Rule 2: the empty count badge is the same width as the one
                    beside it, so a count landing never moves its neighbours. */}
                <Badge count>{""}</Badge>
              </Row>
              <Divider />
              <Stack gap={2}>
                <Label>Indeterminate</Label>
                <Meter label="Converting" />
                <Label>Determinate 62%</Label>
                <Meter value={0.62} />
              </Stack>
            </Stack>
          </div>
        </Section>

        <Section title="Controls" note="Native elements underneath, restyled shells on top.">
          <div className="gal__grid2">
            <div className="gal__specimen">
              <Stack gap={4}>
                {/* Label and hint wrap the input in a Field on their own. */}
                <Input
                  label="Custom words"
                  placeholder="Comma separated"
                  defaultValue="Uniwise, Tool-Kit"
                  hint="Words local OCR should prefer when it is unsure."
                />
                <Input
                  label="Backend token"
                  placeholder="Required"
                  error="The service rejected this token."
                />
                <Select
                  label="Output format"
                  defaultValue="markdown"
                  hint="Applies to every file in the run."
                  options={[
                    { value: "markdown", label: "Markdown" },
                    { value: "html", label: "HTML" },
                    { value: "json", label: "JSON" },
                  ]}
                />
              </Stack>
            </div>
            <div className="gal__specimen">
              <Stack gap={4}>
                <Switch
                  label="Skip files already done"
                  hint="The hint drops to its own full-width line under the control."
                  defaultChecked
                />
                <Switch label="High accuracy" />
                <Switch label="Disabled setting" disabled />
                <Divider />
                <Input label="Disabled" defaultValue="Locked" disabled />
                {/* Field + bare Input, for a control that shares its row. */}
                <Field label="API key" htmlFor="gal-key">
                  <Row gap={2}>
                    <Input id="gal-key" type="password" placeholder="••••••••" />
                    <Button>Save</Button>
                  </Row>
                </Field>
              </Stack>
            </div>
          </div>
        </Section>

        <Section title="Modes" note="Mutually exclusive choices, and sections that fold away.">
          <div className="gal__specimen">
            <Stack gap={4}>
              <Segmented
                label="Job"
                value={job}
                onChange={setJob}
                options={[
                  { value: "convert", label: "Convert", count: 6 },
                  { value: "transcribe", label: "Transcribe", count: 0 },
                ]}
              />
              <Segmented
                label="Document mode"
                size="sm"
                value={job}
                onChange={setJob}
                options={[
                  { value: "convert", label: "Read" },
                  { value: "transcribe", label: "Edit" },
                ]}
              />
            </Stack>
          </div>
        </Section>

        <Section title="Surfaces" note="A panel is a titled region; a well is a recessed one.">
          <Stack gap={4}>
            <div className="gal__grid2">
              <Panel title="Queue" actions={<Badge count>6</Badge>}>
                <Stack gap={3}>
                  {(
                    [
                      { name: "board-deck.pdf", tone: "pass" },
                      { name: "tax-return.pdf", tone: "live" },
                      { name: "scan_0043.pdf", tone: "fault" },
                    ] as const
                  ).map(({ name, tone }) => (
                    <Row key={name} gap={3}>
                      <StatusDot tone={tone} label={tone} />
                      <Meta truncate>{name}</Meta>
                      <Spacer />
                      <Meta size="xs" tone="ghost">
                        00:12
                      </Meta>
                    </Row>
                  ))}
                </Stack>
              </Panel>

              <Panel title="Output">
                <Well>
                  <Text size="sm" tone="muted">
                    # Board deck
                    <br />
                    <br />
                    Revenue grew 24% year over year.
                  </Text>
                </Well>
              </Panel>
            </div>
          </Stack>
        </Section>

        <Section
          title="Workspace"
          note="The document pane's parts. Tabs own a panel, so unlike Segmented they are a real tablist."
        >
          <Stack gap={5}>
            <Stack gap={2}>
              <Label>Tabs</Label>
              <Panel>
                <Tabs
                  label="Open documents"
                  value={tab}
                  onChange={setTab}
                  onClose={(id) => { setClosed((c) => [...c, id]); }}
                  items={[
                    { id: "agreement", label: "Schild-Strategic-Partner-Agreement.md", dirty: true },
                    { id: "crosswalk", label: "NIST-800-171-crosswalk.md" },
                    { id: "deck", label: "Q3-board-deck.md" },
                  ].filter((t) => !closed.includes(t.id))}
                />
              </Panel>
              <Text size="xs" tone="faint">
                Arrows move and activate, Delete closes. The dirty dot and the close
                control share one slot, so a tab never changes width.
                {closed.length > 0 ? ` Closed: ${closed.join(", ")}` : ""}
              </Text>
            </Stack>

            <Stack gap={2}>
              <Label>Source editor</Label>
              <Panel>
                <div style={{ height: 260 }}>
                  <SourceEditor
                    value={source}
                    onChange={setSource}
                    label="Sample document source"
                  />
                </div>
              </Panel>
              <Text size="xs" tone="faint">
                Line numbers stay aligned under soft wrap, which is the whole reason
                this is CodeMirror and not a textarea. Highlighting spends no colour:
                structure is drawn with the text ramp and weight, so the status
                colours keep meaning exactly one thing each.
              </Text>
            </Stack>

            <Stack gap={2}>
              <Label>Path</Label>
              <Panel>
                <Stack gap={2}>
                  <Path path="~/Desktop/notes.md" />
                  <Path path="/Users/me/Documents/Work/2026/Q3/quarterly-report.md" />
                  <div style={{ width: 190 }}>
                    <Path path="~/Documents/Work/2026/Q3/quarterly-report.md" />
                  </div>
                </Stack>
              </Panel>
              <Text size="xs" tone="faint">
                The middle elides, never the ends: you scan for the file and read the
                trail only when you need it, so the file is the crumb that must
                survive. The last row is the same path in 190px, where the trail
                gives way first because it shrinks a hundred times faster. Hover for
                the full path.
              </Text>
            </Stack>

            <Stack gap={2}>
              <Label>Split pane</Label>
              <SplitPaneSpecimen />
              <Text size="xs" tone="faint">
                Drag the seam, or focus it and use the arrow keys. The hairline stays
                a hairline at rest and only brightens under the pointer. The
                document never takes less than half. In a specimen this narrow the
                start pane's 240px floor wins over the default share, which is the
                floor doing its job, and the ratio below is what the last drag
                reported.
              </Text>
            </Stack>
          </Stack>
        </Section>

        <Section
          title="Toast"
          note="The one channel for a message that reaches no row. The tone is the left rule and nothing else."
        >
          <div className="gal__grid2">
            {/* Each host is its own containing block, or both bars would stack
                at the foot of the window. */}
            <div className="gal__toasthost">
              <Toast>Output folder set to ~/Deals/Acme.</Toast>
            </div>
            <div className="gal__toasthost">
              <Toast tone="danger">The service rejected this token.</Toast>
            </div>
          </div>
        </Section>

        <Section
          title="Sheet"
          note="A native dialog in the top layer. Escape closes it, Tab stays inside it, and the toast comes through the overlay slot."
        >
          <div className="gal__specimen">
            <SheetSpecimen />
          </div>
        </Section>

        <Section
          title="Tree"
          note="Finder's key map. Arrows select, Right descends, Left climbs, Option takes the subtree, Space inspects, typing jumps."
        >
          <div className="gal__specimen">
            <TreeSpecimen />
          </div>
        </Section>
      </div>
    </div>
  );
}

/// The sheet, its drag strip, and the toast that has to draw above it. A fixed
/// toast rendered outside the dialog sits under the top layer and is invisible,
/// so the Toast primitive goes through the overlay slot instead.
function SheetSpecimen() {
  const [open, setOpen] = useState(false);
  const [toast, setToast] = useState<string | null>(null);

  useEffect(() => {
    if (toast === null) return;
    const timer = setTimeout(() => {
      setToast(null);
    }, 2400);
    return () => {
      clearTimeout(timer);
    };
  }, [toast]);

  return (
    <div className="gal__sheethost">
      <Row gap={3}>
        <Button
          variant="primary"
          onClick={() => {
            setOpen(true);
          }}
        >
          Open the sheet
        </Button>
        <Text size="xs" tone="faint">
          A card clear of the window on all four sides, holding below the title-bar band so the
          traffic lights stay live. The window behind it goes inert, and its close control sits in
          the title row rather than in a band of its own.
        </Text>
      </Row>
      <Sheet
        open={open}
        onClose={() => {
          setOpen(false);
        }}
        title="Settings"
        head={<div className="gal__sheetdrag" />}
        titleActions={
          <Button
            size="sm"
            variant="ghost"
            aria-label="Close the sheet"
            onClick={() => {
              setOpen(false);
            }}
          >
            Close
          </Button>
        }
        overlay={toast !== null && <Toast>{toast}</Toast>}
      >
        <div className="gal__sheetbody">
          <Stack gap={4}>
            <Input label="Backend token" placeholder="••••••••" />
            <Select
              label="Output format"
              defaultValue="markdown"
              options={[
                { value: "markdown", label: "Markdown" },
                { value: "html", label: "HTML" },
              ]}
            />
            <Row gap={3}>
              <Button
                onClick={() => {
                  setToast("Key saved");
                }}
              >
                Save key
              </Button>
              <Text size="xs" tone="faint">
                Fires a toast through the overlay slot, which is the one place it draws above the
                card.
              </Text>
            </Row>
          </Stack>
        </div>
      </Sheet>
    </div>
  );
}

interface SpecNode {
  path: string;
  name: string;
  /// Present on a branch, empty on an open folder with nothing in it.
  children?: SpecNode[];
  /// Children asked for and not arrived.
  busy?: boolean;
  /// The sibling result this source already has.
  result?: string;
  /// A convertible with no result yet.
  convertible?: boolean;
  /// Entries past the listing cap.
  more?: number;
}

const TREE_SPEC: SpecNode[] = [
  {
    path: "Drop Box",
    name: "Drop Box",
    children: [
      { path: "Drop Box/welcome.md", name: "welcome.md" },
      { path: "Drop Box/board-deck.pdf", name: "board-deck.pdf", result: "board-deck.md" },
      /* A source longer than the pane, paired. The trailing slot says the
         result's kind, so the name keeps the row it is about. */
      {
        path: "Drop Box/2022_NASA_Technology_Roadmap.pdf",
        name: "2022_NASA_Technology_Roadmap.pdf",
        result: "2022_NASA_Technology_Roadmap.md",
      },
      { path: "Drop Box/scan_0043.pdf", name: "scan_0043.pdf", convertible: true },
    ],
  },
  {
    path: "Acme",
    name: "Acme",
    children: [
      {
        path: "Acme/Contracts",
        name: "Contracts",
        children: [
          {
            path: "Acme/Contracts/2026",
            name: "2026",
            children: [
              { path: "Acme/Contracts/2026/msa.pdf", name: "msa.pdf", result: "msa.md" },
              { path: "Acme/Contracts/2026/sow.docx", name: "sow.docx", convertible: true },
            ],
          },
        ],
      },
      { path: "Acme/Calls", name: "Calls", busy: true, children: [] },
      { path: "Acme/Archive", name: "Archive", children: [] },
    ],
  },
  {
    path: "Research",
    name: "Research",
    more: 486,
    children: [
      { path: "Research/img2.png", name: "img2.png", convertible: true },
      { path: "Research/img10.png", name: "img10.png", convertible: true },
    ],
  },
  {
    path: "Legal",
    name: "Legal",
    children: [{ path: "Legal/nda.pdf", name: "nda.pdf", convertible: true }],
  },
];

/// Every branch under `path`, for Option+click and Option+Right.
function subtree(nodes: SpecNode[], path: string): string[] {
  const found: string[] = [];
  const walk = (list: SpecNode[], inside: boolean) => {
    for (const n of list) {
      const within = inside || n.path === path;
      if (within && n.children !== undefined && n.path !== path) found.push(n.path);
      if (n.children) walk(n.children, within);
    }
  };
  walk(nodes, false);
  return found;
}

/// Every tree state at once, at the pane's 240px floor.
function TreeSpecimen() {
  const [open, setOpen] = useState(
    () =>
      new Set([
        "Drop Box",
        "Acme",
        "Acme/Contracts",
        "Acme/Contracts/2026",
        "Acme/Calls",
        "Acme/Archive",
        "Research",
      ]),
  );
  const [selected, setSelected] = useState("Drop Box/board-deck.pdf");
  const [last, setLast] = useState("selected board-deck.pdf");

  const toggle = (path: string, next: boolean, deep: boolean) => {
    setOpen((prev) => {
      const paths = new Set(prev);
      for (const p of [path, ...(deep ? subtree(TREE_SPEC, path) : [])]) {
        if (next) paths.add(p);
        else paths.delete(p);
      }
      return paths;
    });
    setLast(`${next ? "opened" : "closed"} ${path}${deep ? " and its subtree" : ""}`);
  };

  const rows = (nodes: SpecNode[], depth: number): ReactNode =>
    nodes.map((node) => {
      const branch = node.children !== undefined;
      const isOpen = branch && open.has(node.path);
      return (
        <TreeRow
          key={node.path}
          path={node.path}
          depth={depth}
          open={branch ? isOpen : undefined}
          busy={node.busy}
          selected={selected === node.path}
          icon={branch ? <FolderGlyph /> : <FileGlyph />}
          title={node.result === undefined ? node.name : `${node.name} → ${node.result}`}
          end={
            node.result !== undefined ? (
              <Meta size="xs" tone="ghost">
                {node.result.split(".").pop()?.toUpperCase()}
              </Meta>
            ) : node.convertible === true ? (
              /* The in-row control: out of the tab order, because Enter on the
                 row is the keyboard route to the same thing. */
              <Button variant="ghost" size="sm" tabIndex={-1}>
                Convert
              </Button>
            ) : undefined
          }
          group={isOpen && node.busy !== true ? group(node, depth + 1) : undefined}
        >
          {node.name}
        </TreeRow>
      );
    });

  const group = (node: SpecNode, depth: number): ReactNode => {
    const children = node.children ?? [];
    if (children.length === 0) {
      return (
        <TreeRow quiet path={`${node.path}/·empty`} depth={depth}>
          Empty
        </TreeRow>
      );
    }
    return (
      <>
        {rows(children, depth)}
        {node.more !== undefined && (
          <TreeRow quiet path={`${node.path}/·more`} depth={depth}>
            {node.more} more files
          </TreeRow>
        )}
      </>
    );
  };

  return (
    <Stack gap={3}>
      <div className="gal__tree">
        <Tree
          label="Specimen workspace"
          onSelect={(path) => {
            setSelected(path);
            setLast(`selected ${path}`);
          }}
          onActivate={(path) => {
            setLast(`activated ${path}`);
          }}
          onInspect={(path) => {
            setLast(`inspected ${path}`);
          }}
          onToggle={toggle}
        >
          {rows(TREE_SPEC, 0)}
        </Tree>
      </div>
      <Meta size="xs" tone="ghost">
        {last}
      </Meta>
      <Text size="xs" tone="faint">
        Click a row, then drive it from the keyboard. The arrows select and open nothing, Right
        descends into a folder that is already open, Left climbs from a file, Option+Right takes the
        whole subtree, Space inspects without scrolling the pane, and typing jumps. Selection is a
        surface step, never a colour: an ink fill is the control you press, and a selected row is a
        statement of place.
      </Text>
    </Stack>
  );
}

/// Monochrome: a coloured folder is the only colour carrying no signal.
function FolderGlyph() {
  return (
    <svg
      viewBox="0 0 24 24"
      width="13"
      height="13"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.75"
      strokeLinejoin="round"
    >
      <path d="M3 6.5A1.5 1.5 0 0 1 4.5 5h4l2 2.5h7A1.5 1.5 0 0 1 19 9v8.5a1.5 1.5 0 0 1-1.5 1.5h-13A1.5 1.5 0 0 1 3 17.5Z" />
    </svg>
  );
}

function FileGlyph() {
  return (
    <svg
      viewBox="0 0 24 24"
      width="13"
      height="13"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.75"
      strokeLinejoin="round"
    >
      <path d="M6 3.5h7L18 8.5v12H6Z" />
      <path d="M13 3.5V9h5" />
    </svg>
  );
}

/// The seam, and the layout the last drag reported.
function SplitPaneSpecimen() {
  const [layout, setLayout] = useState<SplitLayout>();
  return (
    <Stack gap={2}>
      <Panel>
        <div style={{ height: 160 }}>
          <SplitPane
            layout={layout}
            onLayoutChanged={setLayout}
            start={
              <div className="gal__splitpane">
                <Text size="xs" tone="faint">Run column</Text>
              </div>
            }
            end={
              <div className="gal__splitpane">
                <Text size="xs" tone="faint">Document</Text>
              </div>
            }
          />
        </div>
      </Panel>
      <Meta size="sm" tone={layout ? "default" : "ghost"}>
        {layout ? `${layout.start.toFixed(0)} / ${layout.end.toFixed(0)}` : "-- / --"}
      </Meta>
    </Stack>
  );
}

function Section({
  title,
  note,
  children,
}: {
  title: string;
  note: string;
  children: ReactNode;
}) {
  return (
    <section className="gal__section">
      <div className="gal__head">
        <Display size="xl">{title}</Display>
        <Text size="xs" tone="faint">
          {note}
        </Text>
      </div>
      {children}
    </section>
  );
}
