import React from "react";
import { useTranslation } from "react-i18next";
import { Slider } from "../../ui/Slider";
import { useSettings } from "../../../hooks/useSettings";

interface VadThresholdProps {
  descriptionMode?: "tooltip" | "inline";
  grouped?: boolean;
}

export const VadThreshold: React.FC<VadThresholdProps> = ({
  descriptionMode = "tooltip",
  grouped = false,
}) => {
  const { t } = useTranslation();
  const { settings, updateSetting } = useSettings();

  const handleChange = (value: number) => {
    updateSetting("vad_threshold", value);
  };

  return (
    <Slider
      value={settings?.vad_threshold ?? 0.3}
      onChange={handleChange}
      min={0.05}
      max={0.8}
      step={0.05}
      label={t("settings.debug.vadThreshold.title")}
      description={t("settings.debug.vadThreshold.description")}
      descriptionMode={descriptionMode}
      grouped={grouped}
      formatValue={(v) => v.toFixed(2)}
    />
  );
};
