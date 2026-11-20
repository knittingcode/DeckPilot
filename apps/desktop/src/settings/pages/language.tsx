import { useId } from "react"

import { SettingsGroup, SettingsSection } from "@/components/settings-layout"
import { useTranslation } from "@/i18n"
import { LANGUAGE_NAMES, SUPPORTED_LANGUAGES } from "@/i18n/config"
import { setLanguagePreference, useLanguagePreference } from "@/i18n/language"
import type { LanguagePreference } from "@/i18n/preference"

/**
 * The app's language, for every window and profile on this computer:
 * the system's language (recommended) or one chosen explicitly. Each
 * language is named in that language, so it can be found in any of them.
 * The change applies at once, without a restart, in every open window.
 */
export function LanguageSettings() {
  const { t } = useTranslation()
  const preference = useLanguagePreference()
  const name = useId()
  const choices: { value: LanguagePreference; label: string; lang?: string }[] = [
    { value: "system", label: t("settings:language.system") },
    ...SUPPORTED_LANGUAGES.map((language) => ({
      value: language,
      label: LANGUAGE_NAMES[language],
      lang: language,
    })),
  ]
  return (
    <SettingsSection
      title={t("settings:language.sectionTitle")}
      description={t("settings:language.sectionDescription")}
    >
      <SettingsGroup role="radiogroup" aria-label={t("settings:language.sectionTitle")}>
        {choices.map((choice) => (
          <label
            key={choice.value}
            className="flex cursor-pointer items-center gap-3 py-2.5 text-sm"
          >
            <input
              type="radio"
              name={name}
              value={choice.value}
              className="accent-primary"
              checked={preference === choice.value}
              onChange={() => {
                void setLanguagePreference(choice.value)
              }}
            />
            <span lang={choice.lang} className="font-medium">
              {choice.label}
            </span>
          </label>
        ))}
      </SettingsGroup>
      <p className="text-caption text-muted-foreground">{t("settings:language.systemHint")}</p>
    </SettingsSection>
  )
}
