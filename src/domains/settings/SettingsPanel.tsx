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
  Switch,
  TextInput,
} from "@ui";
import { commands } from "@/app/commands";
import type { SecretId, SecretStatus, Settings } from "@/app/types";

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
    <main className="flow">
      <Row gap={3}>
        <Label tone="strong">API keys</Label>
        <Spacer />
        <Button variant="ghost" iconOnly icon={<XIcon />} onClick={onClose} aria-label="Close settings" />
      </Row>

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

      <Disclosure open={advanced} onToggle={setAdvanced}>
        Advanced
      </Disclosure>
      {advanced && (
        <>
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
        </>
      )}
    </main>
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
  const [value, setValue] = useState("");
  const inputId = useId();
  const typed = value.trim().length > 0;

  const commit = () => {
    onSave(value.trim());
    setValue("");
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
      <Row gap={2} align="center">
        <Input
          id={inputId}
          type="password"
          placeholder={saved ? "••••••••••••  stored in Keychain" : hint}
          value={value}
          onChange={(e) => {
            setValue(e.target.value);
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
