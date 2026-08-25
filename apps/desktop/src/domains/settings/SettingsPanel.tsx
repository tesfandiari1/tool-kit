import { useId, useState } from "react";
import {
  Badge,
  Button,
  Field,
  Input,
  Mono,
  Segmented,
  Select,
  Stack,
  Switch,
  TextInput,
} from "@ui";
import { commands } from "@/app/commands";
import { type SecretId, type SecretStatus, type Settings } from "@/app/types";

/// Which band of settings the sheet is showing. Session state, deliberately
/// not persisted: a setting is looked up, changed, and left, and reopening on
/// last week's tab is a worse default than reopening on the first one.
type Group = "conversion" | "providers" | "runs" | "advanced";

const GROUPS: { value: Group; label: string }[] = [
  { value: "conversion", label: "Conversion" },
  { value: "providers", label: "Providers" },
  { value: "runs", label: "Runs" },
  { value: "advanced", label: "Advanced" },
];

/// Settings, in four bands behind one nav.
///
/// It used to be every control in one scroll, wrapped in `FlowLayout` — which
/// is the *window column's* layout, so the form carried the window's own
/// margins inside a 620px card, and its scroll region never resolved a height
/// against the sheet body that was already scrolling. The panel is a plain
/// column now, and the sheet body is the only thing that scrolls.
export function SettingsPanel({
  settings,
  secrets,
  appOwnsBackend,
  onPersist,
  onSecrets,
  onToast,
}: {
  settings: Settings;
  secrets: SecretStatus;
  /// The app runs the conversion service itself, so the bearer token is its
  /// own business: it mints one per launch and hands it to nothing but its
  /// child. A token typed here would replace that one and 401 every
  /// conversion until the next launch, so the field is not offered.
  appOwnsBackend: boolean;
  onPersist: (patch: Partial<Settings>) => void;
  onSecrets: (s: SecretStatus) => void;
  onToast: (msg: string) => void;
}) {
  const [group, setGroup] = useState<Group>("conversion");

  const saveKey = async (provider: SecretId, value: string) => {
    try {
      await commands.setSecret(provider, value);
      onSecrets(await commands.secretStatus());
      onToast(value ? "Key saved" : "Key cleared");
    } catch (e) {
      onToast(String(e));
    }
  };

  return (
    <div className="settings">
      {/* Sticky rather than a second flex row, so the sheet body stays the one
          scroll container. Two nested scrollers is what broke the old panel. */}
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
            <Select
              label="Conversion route"
              hint="Direct keeps today's Datalab path. Backend routes each supported file through the local conversion service."
              value={settings.conversionRoute}
              onChange={(e) => {
                onPersist({ conversionRoute: e.target.value === "backend" ? "backend" : "direct" });
              }}
              options={[
                { value: "direct", label: "Direct provider" },
                { value: "backend", label: "Conversion backend" },
              ]}
            />

            {settings.conversionRoute === "backend" && (
              <Stack gap={3} className="settings-nest">
                <Select
                  label="Conversion profile"
                  hint="Standard may use the configured fallback. Local only keeps document bytes on this machine."
                  value={settings.conversionProfile}
                  onChange={(e) => {
                    onPersist({
                      conversionProfile: e.target.value === "local_only" ? "local_only" : "standard",
                    });
                  }}
                  options={[
                    { value: "standard", label: "Standard" },
                    { value: "local_only", label: "Local only" },
                  ]}
                />
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
                />
              </Stack>
            )}
          </Stack>
        )}

        {group === "providers" && (
          <Stack gap={3}>
            <KeyField
              label="Datalab"
              hint="X-API-Key"
              saved={secrets.datalab}
              onSave={(v) => void saveKey("datalab", v)}
            />
            <KeyField
              label="Rev.ai"
              hint="Access token"
              saved={secrets.revai}
              onSave={(v) => void saveKey("revai", v)}
            />
            {!appOwnsBackend && (
              <KeyField
                label="Backend token"
                hint="Bearer token"
                saved={secrets.backend}
                onSave={(v) => void saveKey("backend", v)}
              />
            )}
          </Stack>
        )}

        {group === "runs" && (
          <Stack gap={3}>
            <Switch
              label="Skip files already done"
              hint="Leaves a file alone when its result is still on disk. Edit the file, delete the result, or switch output format and it runs again."
              checked={settings.skipAlreadyDone}
              onChange={(e) => {
                onPersist({ skipAlreadyDone: e.target.checked });
              }}
            />
            <Switch
              label="High-accuracy convert"
              hint="Re-OCRs every page and runs an LLM pass. Best for scans and tables, and slower for more credits."
              checked={settings.datalabHighAccuracy}
              onChange={(e) => {
                onPersist({ datalabHighAccuracy: e.target.checked });
              }}
            />
          </Stack>
        )}

        {group === "advanced" && (
          <Stack gap={3}>
            <Select
              label="Convert output format"
              value={settings.datalabFormat}
              onChange={(e) => {
                onPersist({ datalabFormat: e.target.value });
              }}
              options={[
                { value: "markdown", label: "Markdown (.md)" },
                { value: "html", label: "HTML (.html)" },
                { value: "json", label: "JSON (.json)" },
              ]}
            />
            <PipelineField
              value={settings.datalabPipelineId}
              onCommit={(datalabPipelineId) => {
                onPersist({ datalabPipelineId });
              }}
            />
          </Stack>
        )}
      </div>
    </div>
  );
}

/// Held in local state and committed on blur or Enter, never per keystroke.
///
/// The pipeline id is folded into the history's output-format key, so every
/// intermediate prefix the user types would otherwise be persisted, re-scan
/// the whole input selection from disk, and churn the already-done counts that
/// decide the Run button's label. It would also leave a truncated id in
/// settings.json if the window were hidden mid-edit.
function PipelineField({
  value,
  onCommit,
}: {
  value: string | null;
  onCommit: (value: string | null) => void;
}) {
  const [draft, setDraft] = useState(value ?? "");
  const commit = () => {
    onCommit(draft.trim() || null);
  };
  return (
    <TextInput
      label="Datalab pipeline ID (optional)"
      type="text"
      value={draft}
      placeholder="pl_… blank uses the standard convert API"
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

/// The contract caps every multipart text part at 256 bytes. The list travels
/// as one part, newline joined, so an over-long list fails every job in the run
/// with a 422 that names neither this field nor Settings.
const CUSTOM_WORDS_MAX_BYTES = 256;

function customWordsBytes(words: string[]) {
  return new TextEncoder().encode(words.join("\n")).length;
}

/// A list in the model, one text box in the UI, committed on blur or Enter for
/// the same reason as the pipeline id: a half-typed word is not a word.
function CustomWordsField({
  value,
  onCommit,
}: {
  value: string[];
  onCommit: (value: string[]) => void;
}) {
  const [draft, setDraft] = useState(value.join(", "));
  const commit = () => {
    const words: string[] = [];
    for (const word of draft
      .split(/[,\n]/)
      .map((w) => w.trim())
      .filter(Boolean)) {
      if (customWordsBytes([...words, word]) > CUSTOM_WORDS_MAX_BYTES) break;
      words.push(word);
    }
    setDraft(words.join(", "));
    onCommit(words);
  };
  const left = CUSTOM_WORDS_MAX_BYTES - customWordsBytes(value);
  return (
    <TextInput
      label="Custom words (optional)"
      hint={
        <>
          Words local OCR should prefer when it is unsure. Worth setting for names and jargon it keeps
          getting wrong.{" "}
          <Mono as="span" size="xs" tone="ghost">
            {left} of {CUSTOM_WORDS_MAX_BYTES} bytes left
          </Mono>
        </>
      }
      type="text"
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

function KeyField({
  label,
  hint,
  saved,
  onSave,
}: {
  label: string;
  hint: string;
  saved: boolean;
  onSave: (value: string) => void;
}) {
  const [typed, setTyped] = useState(false);
  const inputId = useId();

  const commit = () => {
    const input = document.getElementById(inputId);
    if (!(input instanceof HTMLInputElement)) return;
    onSave(input.value.trim());
    input.value = "";
    setTyped(false);
  };

  return (
    <Field
      htmlFor={inputId}
      label={
        <>
          {label} {saved && <Badge tone="pass">saved</Badge>}
        </>
      }
    >
      <div className="settings-key">
        <Input
          id={inputId}
          type="password"
          placeholder={saved ? "••••••••••••  stored in Keychain" : hint}
          onChange={(e) => {
            setTyped(e.target.value.trim().length > 0);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" && typed) commit();
          }}
        />
        <div className="settings-key__actions">
          {/* Save is disabled on an empty box: it used to delete the stored key,
              which looked identical to a no-op because of the masked placeholder. */}
          <Button disabled={!typed} onClick={commit}>
            Save
          </Button>
          {saved && !typed && (
            <Button
              variant="ghost"
              onClick={() => {
                onSave("");
              }}
              title={`Remove the ${label} key`}
            >
              Remove
            </Button>
          )}
        </div>
      </div>
    </Field>
  );
}
