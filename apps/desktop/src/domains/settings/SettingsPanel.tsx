import { useEffect, useState } from "react";
import { Button, Input, Meta, Meter, Row, Select, Stack, Switch } from "@ui";
import { type Settings, type SpeechModel } from "@/app/types";

/// One list. A plain column, never `FlowLayout`, whose scroll region cannot
/// resolve a height inside a sheet body that already scrolls.
export function SettingsPanel({
  settings,
  speech,
  onPersist,
  onToast,
}: {
  settings: Settings;
  speech: SpeechModelControl;
  onPersist: (patch: Partial<Settings>) => void;
  onToast: (msg: string) => void;
}) {
  return (
    <Stack gap={3} className="settings">
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
      <SpeechLanguageField
        locale={settings.speechLocale}
        onPick={(speechLocale) => {
          onPersist({ speechLocale });
        }}
        {...speech}
      />
      <SpeakerCountField
        value={settings.speakerCount}
        onCommit={(speakerCount) => {
          onPersist({ speakerCount });
        }}
      />
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

/// `useSpeechModel`'s surface. Undefined `model` is a check in flight, null a
/// build with no audio worker.
interface SpeechModelControl {
  model: SpeechModel | null | undefined;
  progress: number | null;
  failure: { message: string; retry: "check" | "download" } | null;
  refresh: (locale: string | null) => Promise<void>;
  download: (locale: string | null) => Promise<void>;
}

const RECORDINGS_STAY = "Your recordings never leave this Mac.";

/// The language Transcribe hears, and whether Apple's model for it is here.
/// Checked each time Settings opens or the pick changes, because a Transcribe
/// job or another app can install it too. Picking a language that is not here
/// is the consent to download it.
function SpeechLanguageField({
  locale,
  onPick,
  model,
  progress,
  failure,
  refresh,
  download,
}: SpeechModelControl & {
  locale: string | null;
  onPick: (locale: string | null) => void;
}) {
  useEffect(() => {
    void refresh(locale);
  }, [refresh, locale]);
  if (model === null) return null;

  const ours = progress !== null;
  const state = ours ? "downloading" : model?.state;
  let hint = "Checking this Mac.";
  switch (state) {
    case "installed":
      hint = "On this Mac. Transcribe works without an internet connection.";
      break;
    case "downloading":
      hint = ours
        ? `Downloading from Apple, ${String(Math.round(progress * 100))}%. ${RECORDINGS_STAY}`
        : `Downloading from Apple. ${RECORDINGS_STAY}`;
      break;
    case "supported":
      hint = `Not on this Mac yet. It downloads once from Apple. ${RECORDINGS_STAY}`;
      break;
    case "unsupported":
      hint = "Apple has no on-device model for this language. Pick one from the list.";
      break;
    case undefined:
      break;
  }

  const options =
    model === undefined
      ? [{ value: locale ?? "", label: "Checking…" }]
      : [
          { value: "", label: `Same as this Mac: ${model.systemLanguage}` },
          ...model.choices.map((c) => ({
            value: c.locale,
            label: c.language,
            group: c.installed ? "On this Mac" : "Download from Apple",
          })),
        ];
  // A saved language Apple no longer lists still shows as picked.
  if (locale !== null && !options.some((o) => o.value === locale)) {
    options.push({ value: locale, label: locale });
  }

  const pick = (value: string) => {
    const next = value || null;
    onPick(next);
    const target = next ?? model?.systemLocale;
    const choice = model?.choices.find((c) => c.locale === target);
    if (choice && !choice.installed) void download(next);
  };

  return (
    <Stack gap={2}>
      <Select
        label="Transcription language"
        hint={hint}
        error={failure?.message}
        value={locale ?? ""}
        disabled={model === undefined || ours}
        options={options}
        onChange={(e) => {
          pick(e.target.value);
        }}
      />
      {state === "downloading" && (
        <Meter value={progress ?? undefined} label="Language download" />
      )}
      {(failure !== null || state === "supported") && (
        <Row>
          <Button
            size="sm"
            onClick={() => {
              void (failure?.retry === "check" ? refresh(locale) : download(locale));
            }}
          >
            {failure === null ? "Download" : "Try again"}
          </Button>
        </Row>
      )}
    </Stack>
  );
}
