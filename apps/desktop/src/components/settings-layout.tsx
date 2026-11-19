// The layout vocabulary of the settings and of the app's other content
// pages: a page header, sections with a heading, plain groups of setting
// rows separated by hairlines, property lists and inline status. Pages are
// flat; cards are left for things that are functional containers (a plugin
// tile, a configuration panel), never for a single setting or a section.

import { type ComponentProps, type ReactNode, useId } from "react"

import { Label } from "@/components/ui/label"
import { cn } from "@/lib/utils"

/** Title and lead text at the top of a settings page. */
export function SettingsHeader({ title, description }: { title: string; description?: string }) {
  return (
    <header className="grid gap-1">
      <h2 className="text-lg leading-7 font-semibold tracking-tight">{title}</h2>
      {description && <p className="text-sm text-muted-foreground">{description}</p>}
    </header>
  )
}

/**
 * A section's heading, its description and commands on the right. The
 * commands wrap below the heading when the window is narrow.
 */
export function SectionHeader({
  title,
  description,
  actions,
  id,
  as: Heading = "h3",
  className,
}: {
  title: ReactNode
  description?: ReactNode
  actions?: ReactNode
  id?: string
  as?: "h2" | "h3" | "h4"
  className?: string
}) {
  return (
    <div
      data-slot="section-header"
      className={cn("flex flex-wrap items-end justify-between gap-x-4 gap-y-2", className)}
    >
      <div className="grid min-w-0 flex-1 basis-64 gap-0.5">
        <Heading
          id={id}
          data-slot="section-title"
          className="flex items-center gap-2 text-sm leading-6 font-semibold"
        >
          {title}
        </Heading>
        {description && (
          <div className="text-caption text-muted-foreground [&_p+p]:mt-1">{description}</div>
        )}
      </div>
      {actions && <div className="flex shrink-0 flex-wrap items-center gap-2">{actions}</div>}
    </div>
  )
}

/** A titled section of a page: a heading (a landmark for screen readers) and its content. */
export function SettingsSection({
  title,
  description,
  actions,
  className,
  children,
  ...props
}: Omit<ComponentProps<"section">, "title"> & {
  title: ReactNode
  description?: ReactNode
  actions?: ReactNode
}) {
  const id = useId()
  return (
    <section
      data-slot="settings-section"
      aria-labelledby={id}
      className={cn("grid gap-3", className)}
      {...props}
    >
      <SectionHeader id={id} title={title} description={description} actions={actions} />
      {children}
    </section>
  )
}

/** Related rows, separated by hairlines; no box around them. */
export function SettingsGroup({ className, ...props }: ComponentProps<"div">) {
  return (
    <div
      data-slot="settings-group"
      className={cn("grid divide-y border-y [&>*]:min-w-0", className)}
      {...props}
    />
  )
}

/**
 * One setting: its label and description on the left, the control on the
 * right. On a narrow window the control moves under the label instead of
 * being clipped.
 */
export function SettingsRow({
  label,
  description,
  htmlFor,
  children,
  className,
  align = "center",
}: {
  label: ReactNode
  description?: ReactNode
  /** The control the label names. Without it the label is plain text. */
  htmlFor?: string
  /** The control(s). */
  children?: ReactNode
  className?: string
  align?: "center" | "start"
}) {
  return (
    <div
      data-slot="settings-row"
      className={cn(
        "flex flex-wrap justify-between gap-x-6 gap-y-2 py-2.5",
        align === "center" ? "items-center" : "items-start",
        className,
      )}
    >
      <div className="grid min-w-0 flex-1 basis-56 gap-1">
        {htmlFor ? (
          <Label htmlFor={htmlFor} className="leading-5">
            {label}
          </Label>
        ) : (
          <div className="text-sm leading-5 font-medium">{label}</div>
        )}
        {description && <div className="text-caption text-muted-foreground">{description}</div>}
      </div>
      {children && (
        <div className="flex max-w-full min-w-0 shrink-0 flex-wrap items-center gap-2">
          {children}
        </div>
      )}
    </div>
  )
}

/** Label–value pairs, e.g. versions and paths; values can be selected and copied. */
export function PropertyList({ className, ...props }: ComponentProps<"dl">) {
  return (
    <dl
      data-slot="property-list"
      className={cn(
        "grid grid-cols-[minmax(6rem,max-content)_1fr] gap-x-6 gap-y-1.5 text-sm",
        className,
      )}
      {...props}
    />
  )
}

export function Property({
  label,
  children,
  mono = false,
}: {
  label: ReactNode
  children: ReactNode
  /** Paths, identifiers and other machine text. */
  mono?: boolean
}) {
  return (
    <div className="contents">
      <dt className="text-muted-foreground">{label}</dt>
      <dd
        className={cn("min-w-0 select-text", mono && "font-mono text-caption leading-5 break-all")}
      >
        {children}
      </dd>
    </div>
  )
}

export type StatusTone = "positive" | "warning" | "negative" | "neutral"

const DOT: Record<StatusTone, string> = {
  positive: "bg-success",
  warning: "bg-warning",
  negative: "bg-destructive",
  neutral: "bg-muted-foreground/50",
}

/** A small coloured dot and a word: a state without the weight of a badge. */
export function InlineStatus({
  tone = "neutral",
  className,
  children,
  ...props
}: ComponentProps<"span"> & { tone?: StatusTone }) {
  return (
    <span
      data-slot="inline-status"
      data-tone={tone}
      className={cn(
        "inline-flex items-center gap-1.5 text-caption whitespace-nowrap",
        tone === "neutral" ? "text-muted-foreground" : "text-foreground",
        className,
      )}
      {...props}
    >
      <span aria-hidden className={cn("size-1.5 shrink-0 rounded-full", DOT[tone])} />
      {children}
    </span>
  )
}

/**
 * A functional container: an editor or configuration form that belongs
 * together, e.g. the variable being edited. A hairline frame without a
 * shadow; never nested in another panel.
 */
export function Panel({
  title,
  description,
  actions,
  className,
  children,
  ...props
}: Omit<ComponentProps<"div">, "title"> & {
  title: ReactNode
  description?: ReactNode
  actions?: ReactNode
}) {
  const id = useId()
  return (
    <div
      role="group"
      data-slot="panel"
      aria-labelledby={id}
      className={cn("grid rounded-lg border", className)}
      {...props}
    >
      <SectionHeader
        id={id}
        title={title}
        description={description}
        actions={actions}
        className="border-b px-4 py-2.5"
      />
      <div className="grid gap-4 p-4">{children}</div>
    </div>
  )
}
