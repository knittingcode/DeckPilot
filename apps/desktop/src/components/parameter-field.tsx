import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Textarea } from "@/components/ui/textarea"
import type { ParameterSpec, ParameterValue, Project, ResourceRef } from "@/lib/types"

/**
 * Form control for one action parameter, chosen from its declared kind.
 * `projects` are the projects of the profile the action belongs to; project
 * parameters can only refer to those.
 */
export function ParameterField({
  spec,
  value,
  projects = [],
  onChange,
}: {
  spec: ParameterSpec
  value: ParameterValue
  projects?: Project[]
  onChange: (value: ParameterValue) => void
}) {
  const id = `parameter-${spec.key}`
  return (
    <div className="grid gap-2">
      <Label htmlFor={id}>
        {spec.label}
        {spec.required && <span className="text-destructive">*</span>}
      </Label>
      <Control id={id} spec={spec} value={value} projects={projects} onChange={onChange} />
      {spec.description && <p className="text-xs text-muted-foreground">{spec.description}</p>}
    </div>
  )
}

function Control({
  id,
  spec,
  value,
  projects,
  onChange,
}: {
  id: string
  spec: ParameterSpec
  value: ParameterValue
  projects: Project[]
  onChange: (value: ParameterValue) => void
}) {
  const kind = spec.kind
  if (kind.type === "text" && value.type === "text") {
    const props = {
      id,
      value: value.value,
      maxLength: kind.maxLength,
      onChange: (e: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => {
        onChange({ type: "text", value: e.target.value })
      },
    }
    return kind.multiline ? <Textarea {...props} /> : <Input {...props} />
  }
  if (kind.type === "integer" && value.type === "integer") {
    return (
      <Input
        id={id}
        type="number"
        min={kind.min}
        max={kind.max}
        value={value.value}
        onChange={(e) => {
          onChange({ type: "integer", value: Math.trunc(Number(e.target.value)) })
        }}
      />
    )
  }
  if (kind.type === "boolean" && value.type === "boolean") {
    return (
      <input
        id={id}
        type="checkbox"
        className="size-4"
        checked={value.value}
        onChange={(e) => {
          onChange({ type: "boolean", value: e.target.checked })
        }}
      />
    )
  }
  if (kind.type === "choice" && value.type === "choice") {
    return (
      <Select
        value={value.value}
        onValueChange={(choice) => {
          onChange({ type: "choice", value: choice })
        }}
      >
        <SelectTrigger id={id} className="w-56">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {kind.options.map((option) => (
            <SelectItem key={option} value={option}>
              {option}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    )
  }
  if (kind.type === "resource" && value.type === "resource") {
    return <ResourceControl id={id} value={value.value} onChange={onChange} />
  }
  if (kind.type === "project" && value.type === "project") {
    const known = projects.some((p) => p.id === value.value)
    if (projects.length === 0) {
      return (
        <p id={id} className="text-sm text-muted-foreground">
          This profile has no projects yet. Add one on the Projects page.
        </p>
      )
    }
    return (
      <>
        <Select
          value={known ? value.value : ""}
          onValueChange={(project) => {
            onChange({ type: "project", value: project })
          }}
        >
          <SelectTrigger id={id} className="w-64">
            <SelectValue placeholder="Choose a project" />
          </SelectTrigger>
          <SelectContent>
            {projects.map((project) => (
              <SelectItem key={project.id} value={project.id}>
                {project.key} · {project.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {value.value !== "" && !known && (
          <p className="text-xs text-destructive">
            The saved project is not part of this profile. Choose one of its projects.
          </p>
        )}
      </>
    )
  }
  return <p className="text-sm text-destructive">Unsupported parameter type.</p>
}

function ResourceControl({
  id,
  value,
  onChange,
}: {
  id: string
  value: ResourceRef
  onChange: (value: ParameterValue) => void
}) {
  const text = value.type === "fixed" ? value.key : value.role
  const set = (reference: ResourceRef) => {
    onChange({ type: "resource", value: reference })
  }
  return (
    <div className="grid grid-cols-[14rem_1fr] gap-2">
      <Select
        value={value.type}
        onValueChange={(type) => {
          set(type === "fixed" ? { type: "fixed", key: text } : { type: "projectRole", role: text })
        }}
      >
        <SelectTrigger aria-label="Resource source">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="projectRole">Role of active project</SelectItem>
          <SelectItem value="fixed">Fixed resource key</SelectItem>
        </SelectContent>
      </Select>
      <Input
        id={id}
        value={text}
        placeholder={value.type === "fixed" ? "shop.repository" : "repository"}
        onChange={(e) => {
          set(
            value.type === "fixed"
              ? { type: "fixed", key: e.target.value }
              : { type: "projectRole", role: e.target.value },
          )
        }}
      />
    </div>
  )
}
