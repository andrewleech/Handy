import React from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";

interface StreamingToggleProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const StreamingToggle: React.FC<StreamingToggleProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();

    const enabled = getSetting("streaming_enabled") || false;
    const liveTyping = getSetting("streaming_live_typing") || false;

    return (
      <>
        <ToggleSwitch
          checked={enabled}
          onChange={(enabled) => updateSetting("streaming_enabled", enabled)}
          isUpdating={isUpdating("streaming_enabled")}
          label={t("settings.advanced.streamingToggle.label")}
          description={t("settings.advanced.streamingToggle.description")}
          descriptionMode={descriptionMode}
          grouped={grouped}
        />
        {enabled && (
          <ToggleSwitch
            checked={liveTyping}
            onChange={(v) => updateSetting("streaming_live_typing", v)}
            isUpdating={isUpdating("streaming_live_typing")}
            label={t("settings.advanced.streamingToggle.liveTypingLabel")}
            description={t(
              "settings.advanced.streamingToggle.liveTypingDescription",
            )}
            descriptionMode={descriptionMode}
            grouped={grouped}
          />
        )}
      </>
    );
  },
);
