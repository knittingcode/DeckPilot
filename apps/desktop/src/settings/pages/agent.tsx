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

export function AgentSettings() {
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
        label: "Refresh status",
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
                  "The agent status could not be read",
                  e,
                  "Try again; if the agent does not respond, restart it",
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
            Status <AgentBadge />
          </>
        }
        description="The agent owns device communication and action execution and keeps running when this window is closed."
      >
        {agent && !agent.running && (
          <SettingsGroup>
            <AgentStopped />
          </SettingsGroup>
        )}
        {status && (
          <>
            <PropertyList className="border-t pt-3">
              <Property label="Version">
                {status.version} (protocol {status.protocol})
              </Property>
              <Property label="Process">
                PID {status.pid}, started {formatTimestamp(status.startedAt)}
              </Property>
              <Property label="Device backend">{status.backend}</Property>
            </PropertyList>
            <SettingsGroup>
              <SettingsRow
                label="Stop the agent"
                description="Stopping the agent blanks connected devices until it is started again."
              >
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => {
                    run(async () => {
                      await api.stopAgent()
                      setAgent(await api.agentStatus())
                    }, "The agent was not stopped")
                  }}
                >
                  <Square /> Stop agent
                </Button>
              </SettingsRow>
            </SettingsGroup>
          </>
        )}
      </SettingsSection>
      <SettingsSection title="Startup">
        <SettingsGroup>
          <SettingsRow
            label="Start the agent when I log in"
            htmlFor="autostart"
            description={
              autostart === null
                ? running
                  ? "Not available on this computer."
                  : "Start the agent to change this."
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
                    ? "Starting the agent at login was not turned on"
                    : "Starting the agent at login was not turned off",
                )
              }}
            />
          </SettingsRow>
        </SettingsGroup>
      </SettingsSection>
    </>
  )
}
