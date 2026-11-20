import { RefreshCw, Square } from "lucide-react"
import { useEffect, useState } from "react"

import { AgentBadge, AgentCompatibilityNotice, AgentStopped } from "@/components/agent"
import { ErrorNotice } from "@/components/page"
import { Button } from "@/components/ui/button"
import {
  Property,
  PropertyList,
  SettingsGroup,
  SettingsRow,
  SettingsSection,
} from "@/components/settings-layout"
import { Switch } from "@/components/ui/switch"
import { useTitlebar } from "@/hooks/use-titlebar"
import { api } from "@/lib/api"
import { failureMessage } from "@/lib/failure"
import { useAppContext } from "@/lib/app-context"
import { formatTimestamp } from "@/lib/format"
import { useTranslation } from "@/i18n"

export function AgentSettings() {
  const { t } = useTranslation()
  const { agent, setAgent } = useAppContext()
  const [loadedAutostart, setAutostart] = useState<boolean | null>(null)
  const [error, setError] = useState<string | null>(null)

  // The agent manages its own login item.
  const running = agent?.running ?? false
  useEffect(() => {
    if (!running) return
    api.getAutostart().then(setAutostart, () => {
      setAutostart(null)
    })
  }, [running])

  const run = (operation: () => Promise<unknown>, failed: string) => {
    setError(null)
    operation().catch((e: unknown) => {
      setError(failureMessage(failed, e))
    })
  }
  const status = agent?.status
  const [refreshing, setRefreshing] = useState(false)
  useTitlebar({
    actions: [
      {
        id: "agent.refresh",
        label: t("settings:agent.refresh"),
        icon: RefreshCw,
        placement: "primary",
        busy: refreshing,
        disabled: refreshing,
        onSelect: () => {
          setRefreshing(true)
          setError(null)
          api
            .agentStatus()
            .then(setAgent, (e: unknown) => {
              setError(
                failureMessage(
                  t("settings:agent.statusUnreadable"),
                  e,
                  t("settings:agent.statusUnreadableNext"),
                ),
              )
            })
            .finally(() => {
              setRefreshing(false)
            })
        },
      },
    ],
  })
  const autostart = running ? loadedAutostart : null

  return (
    <>
      <AgentCompatibilityNotice />
      {error && <ErrorNotice error={error} />}
      <SettingsSection
        title={
          <>
            {t("settings:agent.status")} <AgentBadge />
          </>
        }
        description={t("settings:agent.description")}
      >
        {agent && !agent.running && (
          <SettingsGroup>
            <AgentStopped />
          </SettingsGroup>
        )}
        {status && (
          <>
            <PropertyList className="border-t pt-3">
              <Property label={t("devices:agent.version")}>
                {t("devices:agent.versionValue", {
                  version: status.version,
                  protocol: status.protocol,
                })}
              </Property>
              <Property label={t("settings:agent.process")}>
                {t("settings:agent.processValue", {
                  pid: status.pid,
                  time: formatTimestamp(status.startedAt),
                })}
              </Property>
              <Property label={t("settings:agent.backend")}>{status.backend}</Property>
            </PropertyList>
            <SettingsGroup>
              <SettingsRow
                label={t("settings:agent.stopLabel")}
                description={t("settings:agent.stopDescription")}
              >
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => {
                    run(async () => {
                      await api.stopAgent()
                      setAgent(await api.agentStatus())
                    }, t("settings:agent.notStopped"))
                  }}
                >
                  <Square /> {t("settings:agent.stop")}
                </Button>
              </SettingsRow>
            </SettingsGroup>
          </>
        )}
      </SettingsSection>
      <SettingsSection title={t("settings:agent.startup")}>
        <SettingsGroup>
          <SettingsRow
            label={t("settings:agent.autostart")}
            htmlFor="autostart"
            description={
              autostart === null
                ? running
                  ? t("settings:agent.autostartUnavailable")
                  : t("settings:agent.autostartNeedsAgent")
                : undefined
            }
          >
            <Switch
              id="autostart"
              disabled={autostart === null}
              checked={autostart ?? false}
              onCheckedChange={(enabled) => {
                run(
                  async () => {
                    setAutostart(await api.setAutostart(enabled))
                  },
                  enabled
                    ? t("settings:agent.autostartNotOn")
                    : t("settings:agent.autostartNotOff"),
                )
              }}
            />
          </SettingsRow>
        </SettingsGroup>
      </SettingsSection>
    </>
  )
}
