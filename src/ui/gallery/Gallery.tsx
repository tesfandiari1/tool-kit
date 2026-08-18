import { useEffect, useState } from "react";
import type { ReactNode } from "react";
import {
  Badge,
  Button,
  Cell,
  CellGrid,
  Disclosure,
  Display,
  Divider,
  Field,
  Input,
  Label,
  Meter,
  Mono,
  Panel,
  Row,
  Segmented,
  Select,
  Spacer,
  Stack,
  Status,
  StatusDot,
  Switch,
  Text,
  TextInput,
  Well,
} from "../index";
import "./Gallery.css";

/// Every primitive in every state, in both themes. This is the review surface
/// for the design system: if a component is not here, it cannot be checked, so
/// adding a primitive means adding its specimen in the same commit.
///
/// Open with `?gallery` in dev, or in the browser preview harness.

const THEME_KEY = "toolkit.gallery.theme";
type Theme = "graphite" | "paper";

export function Gallery() {
  const [theme, setTheme] = useState<Theme>(
    () => (localStorage.getItem(THEME_KEY) as Theme | null) ?? "graphite",
  );
  // Interactive specimens: a segmented control and a disclosure are only
  // reviewable if you can actually operate them.
  const [job, setJob] = useState<"convert" | "transcribe">("convert");
  const [advanced, setAdvanced] = useState(false);

  useEffect(() => {
    const root = document.documentElement;
    if (theme === "paper") root.setAttribute("data-theme", "paper");
    else root.removeAttribute("data-theme");
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
          <Button
            size="sm"
            variant={theme === "graphite" ? "primary" : "quiet"}
            onClick={() => {
              setTheme("graphite");
            }}
          >
            Graphite
          </Button>
          <Button
            size="sm"
            variant={theme === "paper" ? "primary" : "quiet"}
            onClick={() => {
              setTheme("paper");
            }}
          >
            Paper
          </Button>
        </div>

        <Section title="Colour" note="Signal only. Amber live, green passed, red failed.">
          <div className="gal__swatches">
            {(
              [
                ["--surface", "surface"],
                ["--surface-raised", "surface raised"],
                ["--surface-well", "surface well"],
                ["--rule-lit", "rule"],
                ["--ink", "ink"],
                ["--ink-2", "ink 2"],
                ["--ink-3", "ink 3"],
                ["--accent", "accent"],
                ["--accent-quiet", "accent quiet"],
                ["--live", "live"],
                ["--pass", "pass"],
                ["--fault", "fault"],
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

        <Section title="Typography" note="Three families, three jobs, no overlap.">
          <div className="gal__specimen">
            <Stack gap={4}>
              <Display size="3xl">Less time configuring</Display>
              <Display size="2xl">Convert 24 files</Display>
              <Display size="xl">Run history</Display>
              <Divider />
              <Label>Label — the signature</Label>
              <Label tone="strong">Label strong</Label>
              <Label tone="accent">Label accent</Label>
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
              <Mono>~/Deals/Acme/2024-Q3_board-deck.pdf</Mono>
              <Mono tone="ink">00:04:18</Mono>
              <Mono tone="ghost">--:--:--</Mono>
            </Stack>
          </div>
        </Section>

        <Section title="Button" note="Every control wears the mono label treatment.">
          <div className="gal__specimen">
            <Stack gap={4}>
              <Row gap={2} wrap>
                <Button variant="primary">Convert 3 files</Button>
                <Button variant="secondary">Contact sales</Button>
                <Button variant="quiet">Stop</Button>
                <Button variant="ghost">Clear</Button>
                <Button variant="danger">Delete</Button>
              </Row>
              <Row gap={2} wrap>
                <Button variant="primary" size="sm">
                  Small
                </Button>
                <Button variant="quiet" size="sm">
                  Small quiet
                </Button>
                <Button variant="primary" disabled>
                  Disabled
                </Button>
                {/* link: a text action with no box, still label-treated. */}
                <Button variant="link">Clear history</Button>
              </Row>
              {/* The four treatments the app actually ships on its run screen:
                  lg actuator, its outlined busy state, and icon-only chrome. */}
              <Row gap={2} align="stretch" wrap>
                <Button variant="primary" size="lg" icon="▶">
                  Convert 3 files
                </Button>
                <Button variant="primary" size="lg" busy icon="◌">
                  Working… 3 of 6
                </Button>
                <Button variant="quiet" size="lg" icon="■">
                  Stop
                </Button>
              </Row>
              <Row gap={2} wrap>
                <Button variant="ghost" iconOnly icon="⚙" aria-label="Settings" />
                <Button variant="ghost" size="sm" iconOnly icon="◉" aria-label="Preview" />
                <Button variant="quiet" iconOnly icon="⧉" aria-label="Copy" />
              </Row>
              <Button variant="primary" block>
                Block
              </Button>
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
              {/* Status over StatusDot wherever the glyph's shape carries meaning
                  the colour cannot, such as a spinner for work in flight. */}
              <Row gap={4} wrap>
                {(
                  [
                    { tone: "queued", glyph: "◌", name: "queued" },
                    { tone: "live", glyph: "◌", name: "live" },
                    { tone: "pass", glyph: "✓", name: "pass" },
                    { tone: "fault", glyph: "!", name: "fault" },
                    { tone: "idle", glyph: "·", name: "idle" },
                  ] as const
                ).map((s) => (
                  <Row key={s.name} gap={2}>
                    <Status tone={s.tone} label={s.name}>
                      {s.glyph}
                    </Status>
                    <Label>{s.name}</Label>
                  </Row>
                ))}
              </Row>
              <Divider />
              <Row gap={2} wrap>
                <Badge>Universal-3.5 Pro</Badge>
                <Badge tone="accent">Markdown</Badge>
                <Badge tone="live">Running</Badge>
                <Badge tone="pass">Done</Badge>
                <Badge tone="fault">Failed</Badge>
                <Badge square>24</Badge>
              </Row>
              <Divider />
              <Stack gap={2}>
                <Label>Indeterminate</Label>
                <Meter label="Converting" />
                <Label>Determinate 62%</Label>
                <Meter value={0.62} />
                <Label>Passed</Label>
                <Meter value={1} tone="pass" />
                <Label>Failed</Label>
                <Meter value={0.4} tone="fault" />
              </Stack>
            </Stack>
          </div>
        </Section>

        <Section title="Controls" note="Native elements underneath, restyled shells on top.">
          <div className="gal__grid2">
            <div className="gal__specimen">
              <Stack gap={4}>
                <TextInput label="Pipeline ID" placeholder="pl_…" defaultValue="pl_7f2a91" />
                <TextInput
                  label="API key"
                  placeholder="Required"
                  error="Datalab rejected this key."
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
                <TextInput label="Disabled" defaultValue="Locked" disabled />
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
              <Divider />
              <Disclosure open={advanced} onToggle={setAdvanced}>
                Advanced
              </Disclosure>
              {advanced && (
                <Text size="sm" tone="muted">
                  The trigger does not own its content, so a collapsed section skips mounting
                  entirely rather than hiding with CSS.
                </Text>
              )}
            </Stack>
          </div>
        </Section>

        <Section title="Surfaces" note="Cells share hairlines: one lattice, not a row of cards.">
          <Stack gap={4}>
            <CellGrid columns={3}>
              {[
                ["Convert", "PDF, DOCX, images and more into clean Markdown."],
                ["Transcribe", "Audio and video into text, with speaker turns."],
                ["History", "Every result, and where it went."],
              ].map(([title, body]) => (
                <Cell key={title} interactive>
                  <Stack gap={3}>
                    <Display size="lg">{title}</Display>
                    <Text size="sm" tone="muted">
                      {body}
                    </Text>
                    <Row gap={2} wrap>
                      <Badge>Datalab</Badge>
                    </Row>
                  </Stack>
                </Cell>
              ))}
            </CellGrid>

            <div className="gal__grid2">
              <Panel title="Queue" actions={<Badge square>6</Badge>}>
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
                      <Mono truncate>{name}</Mono>
                      <Spacer />
                      <Mono size="xs" tone="ghost">
                        00:12
                      </Mono>
                    </Row>
                  ))}
                </Stack>
              </Panel>

              <Panel tone="raised" title="Output">
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
      </div>
    </div>
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
