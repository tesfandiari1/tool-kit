import { useState } from "react";
import { Input, Meta, Segmented, Stack, Switch } from "@ui";
import { type Settings } from "@/app/types";

/// Session state: reopening on last week's tab is a worse default.
type Group = "conversion" | "runs";

const GROUPS: { value: Group; label: string }[] = [
  { value: "conversion", label: "Conversion" },
  { value: "runs", label: "Runs" },
];

/// Two bands behind one nav. A plain column, never `FlowLayout`, whose scroll
/// region cannot resolve a height inside a sheet body that already scrolls.
export function SettingsPanel({
  settings,
  onPersist,
  onToast,
}: {
  settings: Settings;
  onPersist: (patch: Partial<Settings>) => void;
  onToast: (msg: string) => void;
}) {
  const [group, setGroup] = useState<Group>("conversion");

  return (
    <div className="settings">
      {/* Sticky, so the sheet body stays the one scroll container. */}
      <div className="settings__nav">
        <Segmented
          options={GROUPS}
          value={group}
          onChange={setGroup}
          label="Settings section"
          size="sm"
        />
      </div>

      <div className="settings__body">
        {group === "conversion" && (
          <Stack gap={3}>
            <Switch
              label="OCR language correction"
              hint="Lets local OCR correct what it reads against a dictionary. Turn it off for part numbers, codes, and names it keeps rewriting."
              checked={settings.languageCorrection}
              onChange={(e) => {
                onPersist({ languageCorrection: e.target.checked });
              }}
            />
            <CustomWordsField
              value={settings.customWords}
              onCommit={(customWords) => {
                onPersist({ customWords });
              }}
              onToast={onToast}
            />
            <SpeakerCountField
              value={settings.speakerCount}
              onCommit={(speakerCount) => {
                onPersist({ speakerCount });
              }}
            />
          </Stack>
        )}

        {group === "runs" && (
          <Stack gap={3}>
            <Switch
              label="Skip files already done"
              hint="Leaves a file alone when its result is still on disk. Edit the file or delete the result and it runs again."
              checked={settings.skipAlreadyDone}
              onChange={(e) => {
                onPersist({ skipAlreadyDone: e.target.checked });
              }}
            />
            {settings.workspacePath !== null && (
              <Switch
                label="Move dropped files into the project"
                hint="Off, a dropped file stays where it is and only its result lands in the project. On, the file moves in beside its result. Nothing is ever copied."
                checked={settings.moveDroppedFiles}
                onChange={(e) => {
                  onPersist({ moveDroppedFiles: e.target.checked });
                }}
              />
            )}
          </Stack>
        )}
      </div>
    </div>
  );
}

/// The contract caps a multipart text part at 256 bytes, and this list travels
/// as one, so an over-long list 422s every job in the run.
const CUSTOM_WORDS_MAX_BYTES = 256;

function customWordsBytes(words: string[]) {
  return new TextEncoder().encode(words.join("\n")).length;
}

function parseCustomWords(draft: string) {
  return draft
    .split(/[,\n]/)
    .map((w) => w.trim())
    .filter(Boolean);
}

/// A list in the model, one box in the UI. A half-typed word is not a word.
function CustomWordsField({
  value,
  onCommit,
  onToast,
}: {
  value: string[];
  onCommit: (value: string[]) => void;
  onToast: (msg: string) => void;
}) {
  const [draft, setDraft] = useState(value.join(", "));
  const commit = () => {
    const typed = parseCustomWords(draft);
    const words: string[] = [];
    for (const word of typed) {
      if (customWordsBytes([...words, word]) > CUSTOM_WORDS_MAX_BYTES) break;
      words.push(word);
    }
    setDraft(words.join(", "));
    onCommit(words);
    // The box rewriting itself shorter is not a sign anything was dropped.
    if (words.length < typed.length) {
      onToast(
        `Over ${String(CUSTOM_WORDS_MAX_BYTES)} bytes: kept the first ${String(words.length)} words`,
      );
    }
  };
  // Off the draft: a counter that moves only on commit reads "256 left" right
  // up to the truncation it should warn of.
  const left = CUSTOM_WORDS_MAX_BYTES - customWordsBytes(parseCustomWords(draft));
  return (
    <Input
      label="Custom words (optional)"
      hint={
        <>
          Words local OCR should prefer when it is unsure. Worth setting for names and jargon it keeps
          getting wrong.{" "}
          <Meta as="span" size="xs" tone="ghost">
            {left < 0
              ? `${String(-left)} bytes over`
              : `${String(left)} of ${String(CUSTOM_WORDS_MAX_BYTES)} bytes left`}
          </Meta>
        </>
      }
      type="text"
      invalid={left < 0}
      value={draft}
      placeholder="Comma separated"
      onChange={(e) => {
        setDraft(e.target.value);
      }}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") commit();
      }}
    />
  );
}

/// The transcriber wants a count, not a hint: blank is the only way to ask it
/// to guess. Committed on blur or Enter, so "1" on the way to "12" is not saved.
const SPEAKERS_MIN = 1;
const SPEAKERS_MAX = 20;

function SpeakerCountField({
  value,
  onCommit,
}: {
  value: number | null;
  onCommit: (value: number | null) => void;
}) {
  const [draft, setDraft] = useState(value === null ? "" : String(value));
  /// WebKit reports unparsable text as "", which would clear the saved count.
  const commit = (badInput: boolean) => {
    const typed = draft.trim();
    if (!typed && !badInput) {
      setDraft("");
      onCommit(null);
      return;
    }
    const parsed = Number.parseInt(typed, 10);
    if (Number.isNaN(parsed)) {
      setDraft(value === null ? "" : String(value));
      return;
    }
    const count = Math.min(Math.max(parsed, SPEAKERS_MIN), SPEAKERS_MAX);
    setDraft(String(count));
    onCommit(count);
  };
  return (
    <Input
      label="Speakers"
      hint="How many people speak in the recordings. Leave it blank to let the transcriber guess, which is less accurate."
      type="number"
      min={SPEAKERS_MIN}
      max={SPEAKERS_MAX}
      step={1}
      value={draft}
      onChange={(e) => {
        setDraft(e.target.value);
      }}
      onBlur={(e) => {
        commit(e.currentTarget.validity.badInput);
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter") commit(e.currentTarget.validity.badInput);
      }}
    />
  );
}
