import React, { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands, type TranscriptionProvider as Provider } from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { notify } from "@/stores/noticeStore";
import { Button } from "@/components/ui/Button";
import { Dropdown, type DropdownOption } from "@/components/ui/Dropdown";
import { Input } from "@/components/ui/Input";
import { SettingContainer } from "@/components/ui/SettingContainer";
import { SettingsGroup } from "@/components/ui/SettingsGroup";

/**
 * The Mistral API key, write-only: it can be saved or removed, and the
 * backend only says whether one is saved. The key itself never comes back.
 */
const VoxtralApiKey: React.FC = () => {
  const { t } = useTranslation();
  // null until the credential store has answered.
  const [hasKey, setHasKey] = useState<boolean | null>(null);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let active = true;
    commands.hasVoxtralApiKey().then((saved) => {
      if (active) setHasKey(saved);
    });
    return () => {
      active = false;
    };
  }, []);

  const save = async () => {
    const key = draft.trim();
    if (!key) return;
    setBusy(true);
    const result = await commands.setVoxtralApiKey(key);
    setBusy(false);
    if (result.status === "ok") {
      setDraft("");
      setHasKey(true);
    } else {
      // The backend's reason stays in its log; the user gets a translated one.
      notify({ title: t("settings.models.provider.apiKey.saveFailed") });
    }
  };

  const clear = async () => {
    setBusy(true);
    const result = await commands.clearVoxtralApiKey();
    setBusy(false);
    if (result.status === "ok") {
      setHasKey(false);
    } else {
      notify({ title: t("settings.models.provider.apiKey.clearFailed") });
    }
  };

  return (
    <SettingContainer
      title={t("settings.models.provider.apiKey.title")}
      description={t("settings.models.provider.apiKey.description")}
      descriptionMode="tooltip"
      grouped={true}
      layout="horizontal"
    >
      {hasKey ? (
        <div className="flex items-center gap-2">
          <span className="text-sm text-ink-2">
            {t("settings.models.provider.apiKey.saved")}
          </span>
          <Button variant="secondary" size="sm" onClick={clear} disabled={busy}>
            {t("settings.models.provider.apiKey.clear")}
          </Button>
        </div>
      ) : (
        <form
          className="flex items-center gap-2"
          onSubmit={(event) => {
            event.preventDefault();
            void save();
          }}
        >
          <Input
            type="password"
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            placeholder={t("settings.models.provider.apiKey.placeholder")}
            aria-label={t("settings.models.provider.apiKey.title")}
            autoComplete="off"
            spellCheck={false}
            variant="compact"
            disabled={busy || hasKey === null}
            className="min-w-[220px]"
          />
          <Button
            type="submit"
            variant="secondary"
            size="sm"
            disabled={busy || !draft.trim()}
          >
            {t("settings.models.provider.apiKey.save")}
          </Button>
        </form>
      )}
    </SettingContainer>
  );
};

/** Which engine transcribes: the local model, or Mistral Voxtral (cloud). */
export const TranscriptionProvider: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const provider = getSetting("transcription_provider") ?? "local";

  const options = useMemo<DropdownOption[]>(
    () => [
      {
        value: "local",
        label: t("settings.models.provider.options.local"),
      },
      {
        value: "mistral_voxtral",
        label: t("settings.models.provider.options.mistralVoxtral"),
      },
    ],
    [t],
  );

  return (
    <SettingsGroup title={t("settings.models.provider.title")}>
      <SettingContainer
        title={t("settings.models.provider.title")}
        description={t("settings.models.provider.description")}
        descriptionMode="tooltip"
        grouped={true}
        layout="horizontal"
      >
        <Dropdown
          options={options}
          selectedValue={provider}
          onSelect={(value) =>
            updateSetting("transcription_provider", value as Provider)
          }
          disabled={isUpdating("transcription_provider")}
        />
      </SettingContainer>
      {provider === "mistral_voxtral" && (
        <>
          <VoxtralApiKey />
          <p className="px-4 py-2 text-xs text-ink-3">
            {t("settings.models.provider.cloudNote")}
          </p>
        </>
      )}
    </SettingsGroup>
  );
};
