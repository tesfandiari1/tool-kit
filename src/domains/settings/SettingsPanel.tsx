import { useId, useState } from "react";
import { XIcon } from "@phosphor-icons/react";
import {
  Badge,
  Button,
  Disclosure,
  Field,
  Input,
  Label,
  Row,
  Select,
  Spacer,
  Stack,
  Switch,
  TextInput,
} from "@ui";
import { commands } from "@/app/commands";
import { DEFAULT_BACKEND_URL, type SecretId, type SecretStatus, type Settings } from "@/app/types";
import { FlowLayout } from "@/shell/FlowLayout";

export function SettingsPanel({
  settings,
  secrets,
  onPersist,
  onSecrets,
  onToast,
  onClose,
}: {
  settings: Settings;
  secrets: SecretStatus;
  onPersist: (patch: Partial<Settings>) => void;
  onSecrets: (s: SecretStatus) => void;
  onToast: (msg: string) => void;
  onClose: () => void;
}) {
  const [advanced, setAdvanced] = useState(false);

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
    <FlowLayout
      head={
        <Row gap={2}>
          <Spacer />
          <Button variant="ghost" size="sm" iconOnly icon={<XIcon />} onClick={onClose} aria-label="Close settings" />
        </Row>
      }
    >
      {/* Grouped, so the fields the route reveals read as belonging to it.
          --s2 attaches them to the control, --s3 separates them from each
          other, and the column's own --s4 keeps the group apart from what
          follows. */}
      <Stack gap={2}>
        <Select
          label="Conversion route"
          hint="Direct keeps today's Datalab path. Backend will route each supported file through the local conversion service."
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
          <Stack gap={3}>
            <BackendUrlField
              value={settings.backendUrl}
              onCommit={(backendUrl) => {
                onPersist({ backendUrl });
              }}
            />
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
            <KeyField
              label="Backend token"
              hint="Bearer token"
              saved={secrets.backend}
              onSave={(v) => void saveKey("backend", v)}
            />
          </Stack>
        )}
      </Stack>

      <Stack gap={2}>
        <Label tone="strong">Provider keys</Label>
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
        </Stack>
      </Stack>

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
        hint="Re-OCRs every page and runs an LLM pass. Best for scans and tables; slower and costs more credits."
        checked={settings.datalabHighAccuracy}
        onChange={(e) => {
          onPersist({ datalabHighAccuracy: e.target.checked });
        }}
      />

      <Stack gap={2}>
        <Disclosure open={advanced} onToggle={setAdvanced}>
          Advanced
        </Disclosure>
        {advanced && (
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
      </Stack>
    </FlowLayout>
  );
}

function BackendUrlField({
  value,
  onCommit,
}: {
  value: string;
  onCommit: (value: string) => void;
}) {
  const [draft, setDraft] = useState(value);
  const commit = () => {
    const backendUrl = draft.trim() || DEFAULT_BACKEND_URL;
    setDraft(backendUrl);
    onCommit(backendUrl);
  };

  return (
    <TextInput
      label="Backend URL"
      type="url"
      value={draft}
      placeholder={DEFAULT_BACKEND_URL}
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
      hint={`Words local OCR should prefer when it is unsure. Worth setting for names and jargon it keeps getting wrong. ${String(left)} of ${String(CUSTOM_WORDS_MAX_BYTES)} bytes left.`}
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
      <Row gap={2} align="center" className="key-row">
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
      </Row>
    </Field>
  );
}
