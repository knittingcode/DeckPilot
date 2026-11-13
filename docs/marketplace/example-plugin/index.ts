// Example plugin "acme.github-prs" (design artifact, docs/marketplace/architecture.md).
// It covers the same ground as the built-in github module's pull-requests
// action, so the SDK can be judged against a known Rust implementation.

import {
  definePlugin,
  ok,
  PluginError,
  type BaseContext,
  type PluginContext,
} from "@deckpilot/plugin-sdk"

const API = "https://api.github.com"
const REPOSITORY = /^[A-Za-z0-9-]{1,39}\/[A-Za-z0-9._-]{1,100}$/

type Filter = "all" | "author-me" | "review-requested"

function searchQuery(filter: Filter, repository: string | undefined): string {
  const parts = ["is:pr", "is:open", "archived:false"]
  if (repository) {
    if (!REPOSITORY.test(repository)) {
      throw new PluginError("invalid-repository", `"${repository}" is not owner/name`)
    }
    parts.push(`repo:${repository}`)
  } else if (filter === "all") {
    throw new PluginError("all-needs-repository", '"all" needs a repository')
  }
  if (filter === "author-me") parts.push("author:@me")
  if (filter === "review-requested") parts.push("review-requested:@me")
  return parts.join(" ")
}

function isCached(value: unknown): value is { etag: string; count: number } {
  if (typeof value !== "object" || value === null) return false
  const entry = value as Record<string, unknown>
  return typeof entry.etag === "string" && typeof entry.count === "number"
}

async function countPullRequests(
  ctx: BaseContext,
  filter: Filter,
  repository: string | undefined,
): Promise<{ count: number; url: string }> {
  const query = searchQuery(filter, repository)
  const url = new URL("/search/issues", API)
  url.searchParams.set("q", query)
  url.searchParams.set("per_page", "1")

  const cacheKey = `etag:${query}`
  const stored = await ctx.storage.get(cacheKey)
  const cached = isCached(stored) ? stored : undefined

  const headers: Record<string, string> = {
    Accept: "application/vnd.github+json",
    "X-GitHub-Api-Version": "2022-11-28",
  }
  if (cached) headers["If-None-Match"] = cached.etag

  const response = await ctx.http.fetch(url, {
    method: "GET",
    headers,
    auth: { credential: ctx.credentials.ref("token"), scheme: "bearer" },
    maxBytes: 256 * 1024,
  })

  const web = `https://github.com/pulls?q=${encodeURIComponent(query)}`
  if (response.status === 304 && cached) return { count: cached.count, url: web }
  if (response.status === 401) throw new PluginError("unauthorized", "GitHub refused the token")
  if (response.status === 403 || response.status === 429) {
    const limited = response.status === 429 || response.headers["x-ratelimit-remaining"] === "0"
    if (!limited) throw new PluginError("forbidden", "GitHub refused access")
    const retryAfterSeconds = Number(response.headers["retry-after"] ?? "60")
    throw new PluginError("rate-limited", "GitHub's rate limit was reached", {
      data: { retryAfterSeconds },
      retryable: true,
    })
  }
  if (response.status !== 200) {
    throw new PluginError("github-error", `GitHub answered ${String(response.status)}`)
  }

  const body = await response.json()
  const total =
    typeof body === "object" && body !== null && !Array.isArray(body) ? body.total_count : undefined
  if (typeof total !== "number") {
    throw new PluginError("invalid-response", "unexpected answer from GitHub")
  }
  const etag = response.headers.etag
  if (etag) await ctx.storage.set(cacheKey, { etag, count: total })
  return { count: total, url: web }
}

async function refreshReviewCount(ctx: PluginContext): Promise<void> {
  if (!(await ctx.credentials.has("token"))) return
  try {
    const { count } = await countPullRequests(
      ctx,
      "review-requested",
      ctx.settings.defaultRepository || undefined,
    )
    await ctx.variables.publish("reviewCount", count)
    await ctx.variables.publish("checkedAt", new Date().toISOString())
  } catch (error) {
    // Codes only: messages never contain the token, but logs stay terse.
    ctx.log.warn("background refresh failed", {
      code: error instanceof PluginError ? error.code : "plugin-error",
    })
  }
}

export default definePlugin({
  actions: {
    "pr-count": {
      async run(ctx, params) {
        const repository = params.repository || ctx.settings.defaultRepository || undefined
        ctx.progress(undefined, "asking GitHub")
        const { count, url } = await countPullRequests(ctx, params.filter, repository)
        if (params.filter === "review-requested" && !params.repository) {
          await ctx.variables.publish("reviewCount", count)
        }
        await ctx.device.setState({
          label: String(count),
          indicator: count > 0 ? "warning" : "ok",
        })
        return ok(`${String(count)} pull request${count === 1 ? "" : "s"}`, { count, url })
      },

      async state(ctx, params) {
        const repository = params.repository || ctx.settings.defaultRepository || undefined
        try {
          const { count } = await countPullRequests(ctx, params.filter, repository)
          return { label: String(count), detail: "PRs", indicator: count > 0 ? "warning" : "ok" }
        } catch {
          return { detail: "Error", indicator: "error" }
        }
      },
    },
  },

  async onActivate(ctx) {
    let timer: { dispose(): void } | undefined
    const schedule = () => {
      timer?.dispose()
      timer = ctx.timers.every(ctx.settings.pollMinutes * 60_000, () => refreshReviewCount(ctx))
    }
    schedule()
    ctx.onSettingsChanged(schedule)
    await refreshReviewCount(ctx)
  },

  async onDeactivate(ctx) {
    await ctx.variables.clear()
  },
})
