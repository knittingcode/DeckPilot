// The bundled translations. Every namespace of every language is imported
// statically: the app loads nothing over the network and every window has
// all of them before its first render.

import enActions from "./locales/en/actions.json"
import enCommon from "./locales/en/common.json"
import enDevices from "./locales/en/devices.json"
import enDialogs from "./locales/en/dialogs.json"
import enEditor from "./locales/en/editor.json"
import enErrors from "./locales/en/errors.json"
import enLayouts from "./locales/en/layouts.json"
import enMarketplace from "./locales/en/marketplace.json"
import enNavigation from "./locales/en/navigation.json"
import enNotifications from "./locales/en/notifications.json"
import enPermissions from "./locales/en/permissions.json"
import enPlugins from "./locales/en/plugins.json"
import enSecurity from "./locales/en/security.json"
import enSettings from "./locales/en/settings.json"
import enWorkflows from "./locales/en/workflows.json"
import enWorkspace from "./locales/en/workspace.json"
import plActions from "./locales/pl/actions.json"
import plCommon from "./locales/pl/common.json"
import plDevices from "./locales/pl/devices.json"
import plDialogs from "./locales/pl/dialogs.json"
import plEditor from "./locales/pl/editor.json"
import plErrors from "./locales/pl/errors.json"
import plLayouts from "./locales/pl/layouts.json"
import plMarketplace from "./locales/pl/marketplace.json"
import plNavigation from "./locales/pl/navigation.json"
import plNotifications from "./locales/pl/notifications.json"
import plPermissions from "./locales/pl/permissions.json"
import plPlugins from "./locales/pl/plugins.json"
import plSecurity from "./locales/pl/security.json"
import plSettings from "./locales/pl/settings.json"
import plWorkflows from "./locales/pl/workflows.json"
import plWorkspace from "./locales/pl/workspace.json"

import type { Language, Namespace } from "./config"

/** English: the source language, and the shape the keys are typed from. */
export const en = {
  common: enCommon,
  navigation: enNavigation,
  settings: enSettings,
  devices: enDevices,
  layouts: enLayouts,
  editor: enEditor,
  actions: enActions,
  workflows: enWorkflows,
  workspace: enWorkspace,
  marketplace: enMarketplace,
  plugins: enPlugins,
  permissions: enPermissions,
  dialogs: enDialogs,
  security: enSecurity,
  notifications: enNotifications,
  errors: enErrors,
} satisfies Record<Namespace, object>

const pl = {
  common: plCommon,
  navigation: plNavigation,
  settings: plSettings,
  devices: plDevices,
  layouts: plLayouts,
  editor: plEditor,
  actions: plActions,
  workflows: plWorkflows,
  workspace: plWorkspace,
  marketplace: plMarketplace,
  plugins: plPlugins,
  permissions: plPermissions,
  dialogs: plDialogs,
  security: plSecurity,
  notifications: plNotifications,
  errors: plErrors,
} satisfies Record<Namespace, object>

export const resources: Record<Language, Record<Namespace, object>> = { en, pl }
