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

    return (
      <ToggleSwitch
        checked={enabled}
        onChange={(enabled) => updateSetting("streaming_enabled", enabled)}
        isUpdating={isUpdating("streaming_enabled")}
        label={t("settings.advanced.streamingToggle.label")}
        description={t("settings.advanced.streamingToggle.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      />
    );
  },
);
