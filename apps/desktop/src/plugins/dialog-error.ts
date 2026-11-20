import { i18n } from "@/i18n"

import { pluginErrorText, type PluginErrorText } from "./api"

/** A plugin error's text as one sentence: what it means, the agent's explanation, what to do. */
export function errorSentence(text: PluginErrorText, withHint = true): string {
  const { title, message, hint } = text
  return withHint && hint
    ? i18n.t("plugins:errors.sentenceWithHint", { title, message, hint })
    : i18n.t("plugins:errors.sentence", { title, message })
}

/**
 * A plugin error as one sentence for a dialog's inline error: what it
 * means, the agent's explanation, and what the user can do.
 */
export function pluginErrorSentence(error: unknown): string {
  return errorSentence(pluginErrorText(error))
}
