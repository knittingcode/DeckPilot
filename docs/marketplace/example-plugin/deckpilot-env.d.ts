// What `deckpilot-plugin codegen` writes for manifest.json. Generated; do not edit.
// Design artifact: see docs/marketplace/architecture.md section 6.

declare module "@deckpilot/plugin-sdk" {
  interface GeneratedManifest {
    actions: {
      "pr-count": {
        params: {
          repository?: string
          filter: "all" | "author-me" | "review-requested"
        }
        outputs: { count: number; url: string }
      }
    }
    variables: { reviewCount: number; checkedAt: string }
    settings: { pollMinutes: number; defaultRepository?: string }
    credentials: "token"
  }
}

export {}
