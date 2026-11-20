import { type TranslationKey, useTranslation } from "@/i18n"
import { CAPABILITIES } from "@/lib/capabilities"
import type { ActionEffect, ActionType, ParameterKind } from "@/lib/types"

import type { Lookup } from "./describe"

const KIND_LABEL: Record<ParameterKind["type"], TranslationKey> = {
  text: "workflows:inspector.docs.kind.text",
  integer: "workflows:inspector.docs.kind.integer",
  boolean: "workflows:inspector.docs.kind.boolean",
  choice: "workflows:inspector.docs.kind.choice",
  resource: "workflows:inspector.docs.kind.resource",
  project: "workflows:inspector.docs.kind.project",
  lines: "workflows:inspector.docs.kind.lines",
  pairs: "workflows:inspector.docs.kind.pairs",
  code: "workflows:inspector.docs.kind.code",
  credential: "workflows:inspector.docs.kind.credential",
}

/** How the catalog and the inspector describe what an action does. */
const EFFECT_LABEL: Record<ActionEffect, TranslationKey> = {
  readOnly: "workflows:inspector.docs.effect.readOnly",
  opens: "workflows:inspector.docs.effect.opens",
  modifies: "workflows:inspector.docs.effect.modifies",
}

function Badge({ children, muted }: { children: string; muted?: boolean }) {
  return (
    <span
      className={
        muted
          ? "rounded border px-1.5 py-0.5 text-[0.6875rem] text-muted-foreground"
          : "rounded bg-muted px-1.5 py-0.5 text-[0.6875rem]"
      }
    >
      {children}
    </span>
  )
}

/**
 * What a node type is, from the module's declarations only: its module and
 * the permissions that module holds, how it behaves (node spec) and its
 * inputs. The editor shows metadata; it never runs module code.
 */
export function NodeDocumentation({ type, lookup }: { type: ActionType; lookup: Lookup }) {
  const { t } = useTranslation()
  const module = lookup.modules?.find((m) => m.actionTypes.some((t) => t.id === type.id))
  const spec = type.node
  return (
    <section
      className="grid gap-2 border-t px-4 py-3"
      aria-label={t("workflows:inspector.docs.title")}
    >
      <h3 className="text-xs font-medium text-muted-foreground uppercase">
        {t("workflows:inspector.docs.title")}
      </h3>
      <p className="text-xs">
        <code className="font-mono">{type.id}</code>
        {spec && (
          <span className="text-muted-foreground">
            {" "}
            · {t("workflows:inspector.docs.version", { version: spec.version })}
          </span>
        )}
        {module && (
          <span className="text-muted-foreground">
            {" "}
            · {t("workflows:inspector.docs.module", { name: module.name })}
          </span>
        )}
      </p>
      {spec && (
        <div className="flex flex-wrap gap-1">
          <Badge muted={(spec.effect ?? "readOnly") === "readOnly"}>
            {t(EFFECT_LABEL[spec.effect ?? "readOnly"])}
          </Badge>
          <Badge muted={!spec.retrySafe}>
            {spec.retrySafe
              ? t("workflows:inspector.docs.retrySafe")
              : t("workflows:inspector.docs.notRetrySafe")}
          </Badge>
          {spec.idempotent && <Badge>{t("workflows:inspector.docs.idempotent")}</Badge>}
          <Badge muted={!spec.cancellable}>
            {spec.cancellable
              ? t("workflows:inspector.docs.cancellable")
              : t("workflows:inspector.docs.notCancellable")}
          </Badge>
          {spec.changesContext && <Badge>{t("workflows:inspector.docs.selectsProject")}</Badge>}
        </div>
      )}
      {module && module.capabilities.length > 0 && (
        <div className="grid gap-1">
          <h4 className="text-xs font-medium">{t("workflows:inspector.docs.permissions")}</h4>
          <ul className="grid gap-0.5 text-xs text-muted-foreground">
            {module.capabilities.map((capability) => (
              <li key={capability}>{t(CAPABILITIES[capability].title)}</li>
            ))}
          </ul>
        </div>
      )}
      {type.parameters.length > 0 && (
        <div className="grid gap-1">
          <h4 className="text-xs font-medium">{t("workflows:inspector.docs.inputs")}</h4>
          <ul className="grid gap-0.5 text-xs">
            {type.parameters.map((p) => (
              <li key={p.key} className="flex gap-2">
                <code className="font-mono">{p.key}</code>
                <span className="flex-1 text-muted-foreground">
                  {p.label}
                  {p.required ? "" : ` ${t("workflows:inspector.docs.optional")}`}
                </span>
                <span className="text-muted-foreground">{t(KIND_LABEL[p.kind.type])}</span>
              </li>
            ))}
          </ul>
        </div>
      )}
      {spec && spec.outputs.length > 0 && (
        <div className="grid gap-1">
          <h4 className="text-xs font-medium">{t("workflows:inspector.docs.outputs")}</h4>
          <ul className="grid gap-0.5 text-xs">
            {spec.outputs.map((o) => (
              <li key={o.key} className="flex gap-2">
                <code className="font-mono">{o.key}</code>
                <span className="flex-1 text-muted-foreground">
                  {o.label}
                  {o.always ? "" : ` ${t("workflows:inspector.docs.sometimes")}`}
                </span>
                {o.sensitive && (
                  <span
                    className="text-muted-foreground"
                    title={t("workflows:inspector.docs.sensitiveHint")}
                  >
                    {t("workflows:inspector.docs.notKept")}
                  </span>
                )}
              </li>
            ))}
          </ul>
        </div>
      )}
    </section>
  )
}
