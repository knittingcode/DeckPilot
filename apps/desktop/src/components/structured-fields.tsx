import { ArrowDown, ArrowUp, KeyRound, Plus, TriangleAlert, X } from "lucide-react"
import { useEffect, useRef, useState } from "react"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Textarea } from "@/components/ui/textarea"
import { api } from "@/lib/api"
import {
  type Pair,
  duplicateRows,
  jsonProblem,
  looksSecret,
  parseLines,
  parsePairs,
  serializeLines,
  serializePairs,
} from "@/lib/structured"
import type { CredentialInfo } from "@/lib/types"

/**
 * An editable list whose items are kept exactly as typed (spaces included)
 * and stored one per line. Shows the exact vector the action receives.
 */
export function ListField({
  id,
  value,
  itemLabel = "Item",
  maxItems,
  maxLength,
  onChange,
}: {
  id: string
  value: string
  itemLabel?: string
  maxItems?: number
  maxLength?: number
  onChange: (value: string) => void
}) {
  // Local rows so an item being typed (or still empty) is not dropped; a
  // value changed elsewhere (undo, another node) replaces them.
  const [items, setItems] = useState<string[]>(() => parseLines(value))
  const emitted = useRef(value)
  useEffect(() => {
    if (value !== emitted.current) {
      emitted.current = value
      setItems(parseLines(value))
    }
  }, [value])
  const update = (next: string[]) => {
    setItems(next)
    emitted.current = serializeLines(next.filter((item) => item.trim() !== ""))
    onChange(emitted.current)
  }
  const kept = items.filter((item) => item.trim() !== "")
  const tooMany = maxItems !== undefined && kept.length > maxItems
  return (
    <div className="grid gap-2" id={id}>
      {items.length === 0 && (
        <p className="text-xs text-muted-foreground">No {itemLabel.toLowerCase()}s.</p>
      )}
      <ol className="grid gap-1.5" aria-label={`${itemLabel}s`}>
        {items.map((item, index) => {
          const tooLong = maxLength !== undefined && item.length > maxLength
          return (
            <li key={index} className="flex items-center gap-1.5">
              <span className="w-5 shrink-0 text-right font-mono text-xs text-muted-foreground">
                {index + 1}
              </span>
              <Input
                aria-label={`${itemLabel} ${index + 1}`}
                aria-invalid={tooLong || item.trim() === "" ? true : undefined}
                className="font-mono"
                value={item}
                onChange={(e) => {
                  // One item never spans lines: a pasted line break is a space.
                  const next = [...items]
                  next[index] = e.target.value.replace(/[\r\n]+/g, " ")
                  update(next)
                }}
              />
              <Button
                type="button"
                variant="ghost"
                size="icon"
                aria-label={`Move ${itemLabel.toLowerCase()} ${index + 1} up`}
                disabled={index === 0}
                onClick={() => {
                  const next = [...items]
                  ;[next[index - 1], next[index]] = [next[index] ?? "", next[index - 1] ?? ""]
                  update(next)
                }}
              >
                <ArrowUp />
              </Button>
              <Button
                type="button"
                variant="ghost"
                size="icon"
                aria-label={`Move ${itemLabel.toLowerCase()} ${index + 1} down`}
                disabled={index === items.length - 1}
                onClick={() => {
                  const next = [...items]
                  ;[next[index + 1], next[index]] = [next[index] ?? "", next[index + 1] ?? ""]
                  update(next)
                }}
              >
                <ArrowDown />
              </Button>
              <Button
                type="button"
                variant="ghost"
                size="icon"
                aria-label={`Remove ${itemLabel.toLowerCase()} ${index + 1}`}
                onClick={() => {
                  update(items.filter((_, i) => i !== index))
                }}
              >
                <X />
              </Button>
            </li>
          )
        })}
      </ol>
      <div>
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={maxItems !== undefined && items.length >= maxItems}
          onClick={() => {
            setItems([...items, ""])
          }}
        >
          <Plus /> Add {itemLabel.toLowerCase()}
        </Button>
      </div>
      {items.some((item) => item.trim() === "") && (
        <p className="text-xs text-muted-foreground">
          Empty {itemLabel.toLowerCase()}s are left out.
        </p>
      )}
      {items.some((item) => maxLength !== undefined && item.length > maxLength) && (
        <p className="text-xs text-destructive">
          {itemLabel}s can have at most {maxLength} characters.
        </p>
      )}
      {tooMany && (
        <p className="text-xs text-destructive">
          At most {maxItems} {itemLabel.toLowerCase()}s.
        </p>
      )}
      {kept.length > 0 && (
        <p className="text-xs break-all text-muted-foreground">
          Passed as: <code className="font-mono">{JSON.stringify(kept)}</code>
        </p>
      )}
    </div>
  )
}

/**
 * Name/value rows (headers, query parameters) stored as one
 * `name<separator>value` per line. Warns about duplicates and names that
 * hold secrets, which belong in a stored credential.
 */
export function PairsField({
  id,
  value,
  separator,
  nameLabel = "Name",
  valueLabel = "Value",
  maxItems,
  caseInsensitive = false,
  noSecrets = false,
  onChange,
}: {
  id: string
  value: string
  separator: string
  nameLabel?: string
  valueLabel?: string
  maxItems?: number
  caseInsensitive?: boolean
  noSecrets?: boolean
  onChange: (value: string) => void
}) {
  const [rows, setRows] = useState<Pair[]>(() => parsePairs(value, separator))
  const emitted = useRef(value)
  useEffect(() => {
    if (value !== emitted.current) {
      emitted.current = value
      setRows(parsePairs(value, separator))
    }
  }, [value, separator])
  const update = (next: Pair[]) => {
    setRows(next)
    emitted.current = serializePairs(
      next.filter((row) => row.name.trim() !== ""),
      separator,
    )
    onChange(emitted.current)
  }
  const duplicates = duplicateRows(rows, caseInsensitive)
  return (
    <div className="grid gap-2" id={id}>
      {rows.length > 0 && (
        <div className="grid grid-cols-[minmax(0,1fr)_minmax(0,1.4fr)_auto] gap-1.5 text-xs text-muted-foreground">
          <span>{nameLabel}</span>
          <span>{valueLabel}</span>
          <span className="w-9" />
        </div>
      )}
      {rows.map((row, index) => {
        const secret = noSecrets && looksSecret(row.name)
        const duplicate = duplicates.has(index)
        return (
          <div key={index} className="grid gap-1">
            <div className="grid grid-cols-[minmax(0,1fr)_minmax(0,1.4fr)_auto] items-center gap-1.5">
              <Input
                aria-label={`${nameLabel} ${index + 1}`}
                aria-invalid={secret || duplicate ? true : undefined}
                className="font-mono"
                value={row.name}
                onChange={(e) => {
                  const next = [...rows]
                  next[index] = { ...row, name: e.target.value.replace(/[\r\n]+/g, "") }
                  update(next)
                }}
              />
              <Input
                aria-label={`${valueLabel} ${index + 1}`}
                className="font-mono"
                value={row.value}
                onChange={(e) => {
                  const next = [...rows]
                  next[index] = { ...row, value: e.target.value.replace(/[\r\n]+/g, " ") }
                  update(next)
                }}
              />
              <Button
                type="button"
                variant="ghost"
                size="icon"
                aria-label={`Remove ${nameLabel.toLowerCase()} ${index + 1}`}
                onClick={() => {
                  update(rows.filter((_, i) => i !== index))
                }}
              >
                <X />
              </Button>
            </div>
            {secret && (
              <p className="flex items-center gap-1 text-xs text-destructive">
                <TriangleAlert className="size-3.5" /> {row.name} holds a secret and cannot be saved
                here. Use a stored credential instead.
              </p>
            )}
            {duplicate && (
              <p className="text-xs text-destructive">{row.name} appears more than once.</p>
            )}
          </div>
        )
      })}
      <div>
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={maxItems !== undefined && rows.length >= maxItems}
          onClick={() => {
            setRows([...rows, { name: "", value: "" }])
          }}
        >
          <Plus /> Add {nameLabel.toLowerCase()}
        </Button>
      </div>
    </div>
  )
}

/** Source text such as a JSON body, with a validity hint and formatting. */
export function CodeField({
  id,
  value,
  language,
  maxLength,
  onChange,
}: {
  id: string
  value: string
  language: string
  maxLength?: number
  onChange: (value: string) => void
}) {
  const json = language === "json"
  const problem = json ? jsonProblem(value) : null
  return (
    <div className="grid gap-1.5">
      <Textarea
        id={id}
        className="min-h-32 font-mono text-xs"
        spellCheck={false}
        maxLength={maxLength}
        value={value}
        aria-invalid={problem ? true : undefined}
        onChange={(e) => {
          onChange(e.target.value)
        }}
      />
      <div className="flex items-center gap-2 text-xs">
        {json && value.trim() !== "" && (
          <span className={problem ? "text-destructive" : "text-muted-foreground"}>
            {problem ? `Not valid JSON: ${problem}` : "Valid JSON"}
          </span>
        )}
        {json && value.trim() !== "" && !problem && (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={() => {
              onChange(JSON.stringify(JSON.parse(value), null, 2))
            }}
          >
            Format
          </Button>
        )}
      </div>
    </div>
  )
}

const NO_CREDENTIAL = "none"

/**
 * Chooses a credential stored on this computer by name. Secrets are never
 * loaded; a name stored in the action but not on this computer stays
 * visible so it can be added in Settings › HTTP.
 */
export function CredentialField({
  id,
  value,
  optional,
  onChange,
}: {
  id: string
  value: string
  optional: boolean
  onChange: (value: string) => void
}) {
  const [stored, setStored] = useState<CredentialInfo[] | null>(null)
  useEffect(() => {
    api.listCredentials().then(setStored, () => {
      setStored([])
    })
  }, [])
  const known = stored?.some((c) => c.id === value) ?? true
  return (
    <div className="grid gap-1.5">
      <Select
        value={value === "" ? NO_CREDENTIAL : value}
        onValueChange={(choice) => {
          onChange(choice === NO_CREDENTIAL ? "" : choice)
        }}
      >
        <SelectTrigger id={id} className="w-72">
          <KeyRound className="size-4 text-muted-foreground" />
          <SelectValue placeholder="Choose a credential" />
        </SelectTrigger>
        <SelectContent>
          {optional && <SelectItem value={NO_CREDENTIAL}>No credential</SelectItem>}
          {value !== "" && !known && (
            <SelectItem value={value}>{value} (not on this computer)</SelectItem>
          )}
          {(stored ?? []).map((credential) => (
            <SelectItem key={credential.id} value={credential.id}>
              {credential.id}
              {credential.description ? ` · ${credential.description}` : ""}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      {stored !== null && stored.length === 0 && (
        <p className="text-xs text-muted-foreground">
          No credentials are stored on this computer. Add them in Settings › HTTP.
        </p>
      )}
      {value !== "" && !known && (
        <p className="text-xs text-destructive">
          “{value}” is not stored on this computer; requests fail until it is added in Settings ›
          HTTP.
        </p>
      )}
    </div>
  )
}
