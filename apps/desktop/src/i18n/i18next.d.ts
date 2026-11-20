// Typed translation keys: `t("common:actions.cancel")` is checked against the
// English resources, the source language (docs/i18n.md).

import "i18next"

import type { NAMESPACES } from "./config"
import type { en } from "./resources"

declare module "i18next" {
  interface CustomTypeOptions {
    // Every namespace, so keys are always written `namespace:key` and any
    // `t` accepts them; at run time the default namespace is `common`,
    // the first.
    defaultNS: typeof NAMESPACES
    resources: typeof en
    returnNull: false
  }
}
