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

/// Session state: reopening on last week's tab is a worse default.
type Group = "conversion" | "providers" | "runs" | "advanced";

const GROUPS: { value: Group; label: string }[] = [
  { value: "conversion", label: "Conversion" },
  { value: "providers", label: "Providers" },
  { value: "runs", label: "Runs" },
  { value: "advanced", label: "Advanced" },
];

/// Four bands behind one nav. A plain column, never `FlowLayout`, whose scroll
/// region cannot resolve a height inside a sheet body that already scrolls.
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
  /// The app mints the service's token per launch, and one typed here would
  /// 401 every conversion until the next one. The field is not offered.
  appOwnsBackend: boolean;
  onPersist: (patch: Partial<Settings>) => void;
  onSecrets: (s: SecretStatus) => void;
  onToast: (msg: string) => void;
}) {
  const [group, setGroup] = useState<Group>("conversion");

  /// Only Datalab reads the format, the pipeline id and high accuracy, and
  /// local-only never reaches it. See `domains/run/routes.ts`.
  const datalabReachable =
    settings.conversionRoute === "direct" || settings.conversionProfile === "standard";
  const groups = datalabReachable ? GROUPS : GROUPS.filter((g) => g.value !== "advanced");

  /// Reports whether the write landed, so a refused key stays in the field.
  const saveKey = async (provider: SecretId, value: string) => {
    try {
      await commands.setSecret(provider, value);
      onSecrets(await commands.secretStatus());
      onToast(value ? "Key saved" : "Key cleared");
      return true;
    } catch (e) {
      onToast(String(e));
      return false;
    }
  };

  return (
    <div className="settings">
      {/* Sticky, so the sheet body stays the one scroll container. */}
      <div className="settings__nav">
        <Segmented
          options={groups}
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
                  onToast={onToast}
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
              onSave={(v) => saveKey("datalab", v)}
            />
            <KeyField
              label="Rev.ai"
              hint="Access token"
              saved={secrets.revai}
              onSave={(v) => saveKey("revai", v)}
            />
            {!appOwnsBackend && (
              <KeyField
                label="Backend token"
                hint="Bearer token"
                saved={secrets.backend}
                onSave={(v) => saveKey("backend", v)}
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
            {datalabReachable && (
              <Switch
                label="High-accuracy convert"
                hint="Re-OCRs every page and runs an LLM pass. Best for scans and tables, and slower for more credits."
                checked={settings.datalabHighAccuracy}
                onChange={(e) => {
                  onPersist({ datalabHighAccuracy: e.target.checked });
                }}
              />
            )}
          </Stack>
        )}

        {group === "advanced" && (
          <Stack gap={3}>
            <Select
              label="Convert output format"
              /* The backend writes Markdown whatever this says, so the label
                 has to admit the format is Datalab's alone. */
              hint="Applies to files Datalab converts. The conversion backend always writes Markdown."
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

/// Committed on blur or Enter, never per keystroke: the pipeline id is folded
/// into the history key, so every prefix would re-scan the whole selection.
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
    <TextInput
      label="Custom words (optional)"
      hint={
        <>
          Words local OCR should prefer when it is unsure. Worth setting for names and jargon it keeps
          getting wrong.{" "}
          <Mono as="span" size="xs" tone="ghost">
            {left < 0
              ? `${String(-left)} bytes over`
              : `${String(left)} of ${String(CUSTOM_WORDS_MAX_BYTES)} bytes left`}
          </Mono>
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

function KeyField({
  label,
  hint,
  saved,
  onSave,
}: {
  label: string;
  hint: string;
  saved: boolean;
  onSave: (value: string) => Promise<boolean>;
}) {
  const [typed, setTyped] = useState(false);
  const inputId = useId();

  const commit = () => {
    const input = document.getElementById(inputId);
    if (!(input instanceof HTMLInputElement)) return;
    // Clear only once the write lands, or a rejected save loses the key.
    void onSave(input.value.trim()).then((ok) => {
      if (!ok) return;
      input.value = "";
      setTyped(false);
    });
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
          {/* Saving nothing deletes the stored key, and the masked
              placeholder makes that look like a no-op. */}
          <Button disabled={!typed} onClick={commit}>
            Save
          </Button>
          {saved && !typed && (
            <Button
              variant="ghost"
              onClick={() => {
                void onSave("");
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
