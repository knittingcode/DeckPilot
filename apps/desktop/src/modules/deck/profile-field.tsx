import { useContext } from "react"

import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { useTranslation } from "@/i18n"
import { AppContext } from "@/lib/app-context"

/** The action type and parameter this control replaces the text field of. */
export const SWITCH_PROFILE_TYPE = "deck.switch-profile"
export const PROFILE_PARAMETER = "profile"

const NONE = "none"

/**
 * The profile a "Switch profile" action switches to: a list of the profiles
 * that exist, stored as the profile's id. A stored profile that no longer
 * exists stays selected and is shown as deleted, so the user sees why the
 * key fails instead of the value silently changing.
 */
export function ProfileField({
  id,
  value,
  onChange,
}: {
  id: string
  value: string
  onChange: (profileId: string) => void
}) {
  const { t } = useTranslation()
  const profiles = useContext(AppContext)?.profiles.profiles ?? []
  const missing = value !== "" && !profiles.some((p) => p.id === value)
  return (
    <div className="grid gap-1">
      <Select
        value={value === "" ? NONE : value}
        onValueChange={(next) => {
          onChange(next === NONE ? "" : next)
        }}
      >
        <SelectTrigger id={id} className="w-56" aria-invalid={missing || undefined}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value={NONE}>{t("actions:deck.profile.none")}</SelectItem>
          {missing && <SelectItem value={value}>{t("actions:deck.profile.deleted")}</SelectItem>}
          {profiles.map((profile) => (
            <SelectItem key={profile.id} value={profile.id}>
              {profile.name}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      {missing && (
        <p className="text-xs text-destructive" role="note">
          {t("actions:deck.profile.missing")}
        </p>
      )}
    </div>
  )
}
