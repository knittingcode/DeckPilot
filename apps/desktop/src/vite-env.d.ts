/// <reference types="vite/client" />

interface ImportMetaEnv {
  /**
   * "1" builds the experimental plugin marketplace into the app
   * (Settings › Extensions › Plugins). Unset in normal and release builds.
   */
  readonly VITE_DECKPILOT_PLUGINS?: string
}
