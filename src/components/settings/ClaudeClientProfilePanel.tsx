import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Switch } from "@/components/ui/switch";
import { Label } from "@/components/ui/label";
import {
  settingsApi,
  type ClaudeClientProfileConfig,
} from "@/lib/api/settings";

export function ClaudeClientProfilePanel() {
  const { t } = useTranslation();
  const [config, setConfig] = useState<ClaudeClientProfileConfig>({
    enabled: false,
    bodyIdentity: false,
  });
  const [isLoading, setIsLoading] = useState(true);

  useEffect(() => {
    settingsApi
      .getClaudeClientProfileConfig()
      .then(setConfig)
      .catch((e) =>
        console.error("Failed to load claude client profile config:", e),
      )
      .finally(() => setIsLoading(false));
  }, []);

  const handleChange = async (updates: Partial<ClaudeClientProfileConfig>) => {
    const prev = config;
    const newConfig = { ...config, ...updates };
    setConfig(newConfig);
    try {
      await settingsApi.setClaudeClientProfileConfig(newConfig);
    } catch (e) {
      console.error("Failed to save claude client profile config:", e);
      toast.error(String(e));
      setConfig(prev);
    }
  };

  if (isLoading) return null;

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div className="space-y-0.5">
          <Label>{t("settings.advanced.claudeClientProfile.enabled")}</Label>
          <p className="text-xs text-muted-foreground">
            {t("settings.advanced.claudeClientProfile.enabledDescription")}
          </p>
        </div>
        <Switch
          checked={config.enabled}
          onCheckedChange={(checked) => handleChange({ enabled: checked })}
        />
      </div>

      <div className="flex items-center justify-between">
        <div className="space-y-0.5">
          <Label
            className={config.enabled ? undefined : "text-muted-foreground"}
          >
            {t("settings.advanced.claudeClientProfile.bodyIdentity")}
          </Label>
          <p className="text-xs text-muted-foreground">
            {config.enabled
              ? t(
                  "settings.advanced.claudeClientProfile.bodyIdentityDescription",
                )
              : t(
                  "settings.advanced.claudeClientProfile.bodyIdentityRequiresHeader",
                )}
          </p>
        </div>
        <Switch
          checked={config.bodyIdentity}
          disabled={!config.enabled}
          onCheckedChange={(checked) => handleChange({ bodyIdentity: checked })}
        />
      </div>

      <ul className="space-y-1 pl-4 text-xs text-muted-foreground list-disc">
        <li>{t("settings.advanced.claudeClientProfile.noteScope")}</li>
        <li>{t("settings.advanced.claudeClientProfile.noteHeaderOnly")}</li>
        <li>
          {t("settings.advanced.claudeClientProfile.noteProviderPriority")}
        </li>
      </ul>
    </div>
  );
}
