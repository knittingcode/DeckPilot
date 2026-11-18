/**
 * English: the reference dictionary. Its shape is the `Dictionary` type, so
 * every other language must provide exactly the same keys.
 *
 * Every statement about the product was checked against the application's
 * code and documentation (see README.md, "Accuracy"). No version number
 * appears here: versions come from GitHub Releases at build time.
 */
export const en = {
  site: {
    name: 'DeckPilot',
    tagline: 'The open-source Stream Deck controller and automation app.',
    description:
      'DeckPilot is an open-source, local-first Stream Deck controller and automation app for macOS, Windows and Linux. Design layouts, build actions and workflows, keep everything encrypted on your computer.',
  },

  a11y: {
    skipToContent: 'Skip to content',
    mainNav: 'Main',
    footerNav: 'Footer',
    openMenu: 'Open menu',
    closeMenu: 'Close menu',
    homeLink: 'DeckPilot, home',
    languageSwitcher: 'Language',
    switchToDark: 'Switch to dark theme',
    switchToLight: 'Switch to light theme',
    breadcrumb: 'Breadcrumb',
    table: 'Table',
    copyCode: 'Copy code',
    copied: 'Copied',
    copyFailed: 'Copy failed',
    marketplaceSite: 'opens the Marketplace site',
  },

  nav: {
    download: 'Download',
    docs: 'Docs',
    screenshots: 'Screenshots',
    plugins: 'Plugins',
    github: 'GitHub',
  },

  footer: {
    about:
      'DeckPilot is an open-source, local-first Stream Deck controller and automation app for macOS, Windows and Linux.',
    product: 'Product',
    resources: 'Resources',
    project: 'Project',
    download: 'Download',
    screenshots: 'Screenshots',
    features: 'What it does',
    security: 'Security and privacy',
    faq: 'FAQ',
    docs: 'User documentation',
    installation: 'Installation',
    troubleshooting: 'Troubleshooting',
    marketplace: 'Plugin Marketplace',
    pluginDevelopers: 'Plugin developer portal',
    github: 'DeckPilot on GitHub',
    releases: 'Releases',
    changelog: 'Changelog',
    contributing: 'Contributing guide',
    securityPolicy: 'Security policy',
    codeOfConduct: 'Code of Conduct',
    license: 'License',
    legal: 'DeckPilot and this site are licensed under Apache-2.0.',
    trademark:
      'Stream Deck is a trademark of its respective owner. DeckPilot is an independent open-source project and is not affiliated with or endorsed by Elgato or Corsair.',
  },

  /** The state of the project, shown wherever a version is mentioned. */
  status: {
    prerelease: 'Pre-release',
    stable: 'Stable release',
    noRelease: 'In development',
    prereleaseBadge: 'Pre-release {version}',
    stableBadge: 'Version {version}',
  },

  home: {
    title: 'DeckPilot: your Stream Deck, your workflow',
    description:
      'DeckPilot is an open-source app for macOS, Windows and Linux that configures a Stream Deck and automates your work with actions and workflows. Local and encrypted.',
    hero: {
      headline: 'Your Stream Deck. Your workflow.',
      lead: 'DeckPilot is an open-source, cross-platform application for configuring Stream Deck devices and automating tasks with actions and workflows. It is extended through sandboxed plugins from a signed Marketplace.',
      download: 'Download DeckPilot',
      plugins: 'Explore Plugins',
      github: 'View on GitHub',
      note: 'Free and open source under Apache-2.0. No account, no cloud, no telemetry.',
    },
    overview: {
      eyebrow: 'Product overview',
      title: 'A control surface for your own work',
      lead: 'Build a library of actions (open an app, check a repository, call an API, switch projects), combine them into workflows, and place them on keys. Profiles keep separate setups apart, and projects let the same key act on whichever project you are working on.',
      parts: [
        {
          title: 'The background agent',
          text: 'Talks to the device, draws the keys, runs actions and workflows, and lives in the system tray. It keeps working when the window is closed.',
        },
        {
          title: 'The configuration app',
          text: 'Where you design layouts, actions, workflows and settings. It connects to the agent and can be opened and closed at any time.',
        },
      ],
      partsLabel: 'DeckPilot has two parts',
      points: [
        {
          title: 'Action-first',
          text: 'Decide what should happen before deciding which key triggers it. One action can sit on several keys, layouts and gestures.',
        },
        {
          title: 'Built around projects',
          text: 'Cycle through your projects from a key; project-aware actions follow the active one. Paths are bound per computer, so profiles stay portable.',
        },
        {
          title: 'Real automation',
          text: 'A visual workflow editor with conditions, parallel branches, loops, retries and timeouts, running in the agent even when the app is closed.',
        },
        {
          title: 'Private by design',
          text: 'The configuration is local and always encrypted. Anything that can change your system is off until you allow it on that computer.',
        },
      ],
    },
    platforms: {
      eyebrow: 'Supported platforms',
      title: 'One codebase for macOS, Windows and Linux',
      lead: 'Installers are published for all three systems. They have not been verified equally: DeckPilot has been used with a real Stream Deck on macOS only, and Windows and Linux are covered by automated tests.',
      items: [
        {
          name: 'macOS',
          arch: 'Apple Silicon and Intel',
          formats: 'Disk image (.dmg)',
          signing: 'Signed with Developer ID and notarized by Apple',
          verification: 'Tested on real hardware',
          verified: true,
        },
        {
          name: 'Windows',
          arch: 'x64',
          formats: 'Setup program (.exe) or MSI package',
          signing: 'Not code-signed yet: SmartScreen warns on first start',
          verification: 'Automated tests only',
          verified: false,
        },
        {
          name: 'Linux',
          arch: 'x64',
          formats: 'AppImage or .deb package',
          signing: 'Packages are not signed; the .deb installs the udev rule for the device',
          verification: 'Automated tests only',
          verified: false,
        },
      ],
      labels: { arch: 'Architecture', formats: 'Installer', signing: 'Signing', verification: 'Verification' },
      cta: 'See the downloads',
    },
    devices: {
      eyebrow: 'Stream Deck device support',
      title: 'One supported model today, stated plainly',
      lead: 'Only the Stream Deck MK.2 has a driver. Other models exist in DeckPilot only as layouts you can design ahead of time; they cannot be connected.',
      columns: { model: 'Model', controls: 'Controls', status: 'Status', detail: 'What that means' },
      rows: [
        {
          model: 'Stream Deck MK.2',
          controls: '15 keys (5 × 3)',
          status: 'Supported',
          supported: true,
          detail: 'Driver included. Tested on real hardware on macOS; automated tests only on Windows and Linux.',
        },
        {
          model: 'Stream Deck XL',
          controls: '32 keys',
          status: 'Layout design only',
          supported: false,
          detail: 'No driver. You can design a layout, but the device cannot be used.',
        },
        {
          model: 'Stream Deck +',
          controls: '8 keys, 4 dials, 4 touch zones',
          status: 'Layout design only',
          supported: false,
          detail: 'No driver. You can design a layout, but the device cannot be used.',
        },
      ],
      caption: 'Stream Deck models and their support status in DeckPilot',
      note: 'Using several devices at once has not been tested with real hardware. Drivers for more models are on the roadmap, without a date.',
      more: 'Connecting a device',
    },
    layouts: {
      eyebrow: 'Profiles and layouts',
      title: 'Separate setups, designed key by key',
      lead: 'A profile is an independent set of actions, workflows, projects and layouts. Layouts belong to a device model and can be designed without the device.',
      items: [
        {
          title: 'Profiles',
          text: 'Keep work, personal and experimental setups apart. Edit one profile while another runs on the device.',
        },
        {
          title: 'Projects and resources',
          text: 'Projects are selected in a cycle; switching never changes Git branches or stops anything. Each computer maps a project’s resources to its own folders and applications.',
        },
        {
          title: 'Pages and folders',
          text: 'Arrange a layout as a tree of pages. Folders open from a key and get a Back key automatically.',
        },
        {
          title: 'Key appearance',
          text: 'Background and text colour, title size and position, and your own PNG or JPEG icons. Keys show live state.',
        },
        {
          title: 'Three gestures per key',
          text: 'A key can react to a single click, a double click and a long press, with timings you can tune per computer or per key.',
        },
        {
          title: 'An editor that forgives',
          text: 'Drag actions onto keys, copy and paste, undo and redo, and revert changes you have not saved.',
        },
      ],
      more: 'Profiles, layouts and gestures in the documentation',
    },
    actions: {
      eyebrow: 'Actions and workflows',
      title: 'From one key press to a whole routine',
      lead: 'Twelve built-in modules provide the action types; you create the actions you need from them. When one action is not enough, combine them into a workflow.',
      modulesTitle: 'Built-in modules',
      modules: [
        { name: 'General', text: 'Log a message, wait' },
        { name: 'Projects', text: 'Cycle and select projects, look up their resources' },
        { name: 'System', text: 'Open applications, pages, files and folders; run commands (off until allowed)' },
        { name: 'Workspace', text: 'Open a project in your editor, open its pages' },
        { name: 'Git', text: 'Read-only status, branch, changes, last commit, remote state' },
        { name: 'Docker', text: 'Containers, Compose status and logs; up, down and restart (off until allowed)' },
        { name: 'HTTP', text: 'Requests with headers, query and JSON or text bodies; health checks' },
        { name: 'GitHub', text: 'Read-only: pull requests, review requests, checks' },
        { name: 'GitLab', text: 'Read-only: merge requests, review requests, pipelines' },
        { name: 'Jira', text: 'Read-only: issues and searches' },
        { name: 'Notifications', text: 'Desktop notifications, workflow summaries' },
        { name: 'Work Session', text: 'Start, check and finish timed work sessions from a key' },
      ],
      workflowsTitle: 'Workflows',
      workflows: [
        'A visual graph editor: conditions, parallel branches, loops, sub-workflows, delays',
        'Retries and timeouts per step, and an error path when something fails',
        'Start from a key, on a schedule, or on an event such as a device connecting',
        'Live progress on the canvas, and a history of runs you can inspect',
        'Results of one step feed the next; shared variables hold values used everywhere',
      ],
      more: 'Read the documentation',
    },
    marketplace: {
      eyebrow: 'Plugin Marketplace',
      badge: 'New',
      title: 'Plugins from a signed Marketplace',
      lead: 'Plugins take DeckPilot beyond its built-in modules: WebAssembly code in a sandbox, a signed registry, and permissions you approve one by one.',
      status:
        'DeckPilot installs plugins from Settings › Extensions › Plugins. The first ones, by DeckPilot, are Air Quality, Markets, Pomodoro, Break & Hydration and Memory; the GitHub, GitLab and Jira integrations and notifications are built-in plugins. The Marketplace site lists them and documents the SDK for developers.',
      explore: 'Explore the Marketplace',
      developers: 'Plugin developer portal',
      points: ['Sandboxed WebAssembly, outside the app’s process', 'A signed registry: no unverified sideloading', 'Every permission needs your consent'],
    },
    agent: {
      eyebrow: 'Background agent',
      title: 'Your keys keep working with the window closed',
      lead: 'The agent is a separate small program. It owns the device and runs your actions and workflows, so the configuration app is only open when you are changing something.',
      points: [
        { title: 'Lives in the tray', text: 'Open the app, switch the active profile, see connected devices, refresh them, or quit, all from the tray menu.' },
        { title: 'Starts when you log in, if you want', text: 'One switch in Settings registers the agent to start at login, for your user account only.' },
        { title: 'App and agent agree on what they support', text: 'A newer app works with an older agent for what both understand, and tells you when the agent needs a restart.' },
        { title: 'Nothing ends silently', text: 'Updates wait for running workflows, and restarting the agent asks before interrupting them.' },
      ],
      more: 'How the background agent works',
    },
    security: {
      eyebrow: 'Security and privacy',
      title: 'Local-first, encrypted, and explicit about permissions',
      lead: 'There is no cloud service, no account, no synchronization and no telemetry. Nothing is sent anywhere unless an action you created does so, with one exception: the update check.',
      points: [
        {
          title: 'Encrypted at rest',
          text: 'The whole configuration is a SQLCipher database (AES-256). The key lives in the operating system’s credential store, or is derived from your password with Argon2id.',
        },
        {
          title: 'Locking',
          text: 'Lock manually or automatically; while locked the device shows “Locked” and runs no actions. Touch ID on macOS, and re-authentication for sensitive operations such as exports.',
        },
        {
          title: 'Explicit permissions',
          text: 'Running commands, starting and stopping containers, and reaching private networks or self-hosted services are off until you allow them on each computer.',
        },
        {
          title: 'Signed updates',
          text: 'Every update package is signed, and the app checks the signature against the key built into it before installing. macOS builds are signed and notarized.',
        },
        {
          title: 'Credentials stay put',
          text: 'Tokens are stored encrypted on the computer, chosen by name in an action, never shown again and never written into an export.',
        },
        {
          title: 'Encrypted backups',
          text: 'An export is encrypted separately with its own password. It is your backup, and the way to move a configuration to another computer.',
        },
      ],
      caveat:
        'DeckPilot is pre-1.0 software. It has not had an independent security review and is not presented as production-ready.',
      more: 'Security model and threat model',
      report: 'Report a vulnerability privately',
    },
    openSource: {
      eyebrow: 'Open-source development',
      title: 'Built in the open',
      lead: 'DeckPilot is written in Rust, with a Tauri 2 desktop app whose interface uses React and TypeScript. The whole source, the release pipeline and the design documents are public.',
      cards: [
        { title: 'Source code', text: 'Read the code, follow the work, and build DeckPilot yourself.', link: 'Open the repository' },
        { title: 'Contribute', text: 'Bug reports, ideas and pull requests are welcome. Start with the contributing guide.', link: 'Read the guide' },
        { title: 'Roadmap', text: 'Drivers for more models, hardware testing on Windows and Linux, Windows code signing.', link: 'See what is planned' },
        { title: 'Apache-2.0', text: 'A permissive license: use it, change it, ship it, with attribution.', link: 'Read the license' },
      ],
    },
    faq: {
      eyebrow: 'FAQ',
      title: 'Questions and answers',
      items: [
        {
          q: 'Is DeckPilot ready for everyday use?',
          a: 'DeckPilot now has stable releases, and the download page offers the latest one. It is still young: it has been used with real hardware on macOS only, and the Windows and Linux builds are not code-signed. Keep an encrypted export of your configuration.',
        },
        {
          q: 'Which Stream Deck models work?',
          a: 'Only the Stream Deck MK.2. The XL and the Stream Deck + exist as layouts you can design, but they have no driver and cannot be connected.',
        },
        {
          q: 'Do I need an account or an internet connection?',
          a: 'No. There is no account, no cloud service and no telemetry. The configuration is stored encrypted on your computer. The app contacts GitHub Releases to check for updates while it is open; you can turn that off.',
        },
        {
          q: 'Is DeckPilot made by or affiliated with Elgato?',
          a: 'No. DeckPilot is an independent open-source project. It is not affiliated with or endorsed by Elgato or Corsair.',
        },
        {
          q: 'Can I install plugins?',
          a: 'Yes: in Settings › Extensions › Plugins, from DeckPilot’s signed registry. Each plugin runs in a sandbox and only with the permissions you agree to.',
        },
        {
          q: 'Why does Windows warn me when I start the installer?',
          a: 'The Windows installers are not code-signed yet, so SmartScreen shows a warning on first start. Compare the file’s SHA-256 checksum with the one on the download page if you want to be sure it is the published file.',
        },
        {
          q: 'How do I move my setup to another computer?',
          a: 'Make an encrypted export in Settings, under Backup & Restore, and import it on the other computer. Paths and other settings of a computer, keys and passwords are not part of an export; you bind them again there.',
        },
        {
          q: 'How do updates work?',
          a: 'While the app is open it checks GitHub Releases, daily by default, and tells you when a newer version exists. Installing automatically is off unless you choose it. To be offered pre-releases, select the Pre-releases channel in Settings, under Updates.',
        },
      ],
    },
  },

  download: {
    title: 'Download DeckPilot',
    metaTitle: 'Download DeckPilot for macOS, Windows and Linux',
    description:
      'Download DeckPilot for macOS, Windows and Linux. Installers, checksums and release notes come straight from the published GitHub release.',
    lead: 'Installers are served by GitHub Releases, the authoritative source. This page lists what the latest published release really contains.',
    release: {
      version: 'Version',
      channel: 'Channel',
      published: 'Published',
      notes: 'Release notes',
      allAssets: 'All files of this release on GitHub',
      checksums: 'SHA256SUMS',
      prereleaseTitle: 'This is a pre-release',
      prereleaseText:
        'DeckPilot has no stable release yet. A pre-release is for trying the app out: expect rough edges, and keep an encrypted export of your configuration.',
      newerPrerelease: 'A newer pre-release, {version}, is also available.',
      newerPrereleaseLink: 'See it on GitHub',
      generated: 'Read from GitHub Releases on {date}.',
    },
    fresh: {
      newer: 'A newer release, {version}, has been published since this page was built.',
      link: 'Get it from GitHub Releases',
    },
    os: { macos: 'macOS', windows: 'Windows', linux: 'Linux' },
    arch: { arm64: 'Apple Silicon', x64: 'x64', intel: 'Intel', arm64Generic: 'ARM64' },
    format: {
      dmg: 'Disk image (.dmg)',
      exe: 'Setup program (.exe)',
      msi: 'MSI package (.msi)',
      appimage: 'AppImage',
      deb: 'Debian package (.deb)',
      rpm: 'RPM package (.rpm)',
    },
    downloadAsset: 'Download',
    size: 'Size',
    sha256: 'SHA-256',
    noSha: 'No checksum is published for this file.',
    signing: {
      title: 'Signing',
      macSigned: 'Signed with an Apple Developer ID and notarized by Apple, so Gatekeeper accepts it.',
      macSignedNotNotarized: 'Signed with an Apple Developer ID, but not notarized.',
      winSigned: 'Code-signed (Authenticode).',
      winUnsigned:
        'Not code-signed. Windows SmartScreen warns on first start: choose “More info”, then “Run anyway”, once you have checked the file.',
      linuxUnsigned: 'The package is not signed. Verify the download with its SHA-256 checksum.',
      unknown: 'This release does not record how this build was signed.',
    },
    notes: {
      macos: 'Open the disk image and drag DeckPilot to Applications.',
      windows: 'Run the setup program, or install the MSI package: both install the same application.',
      linux:
        'The .deb package installs the udev rule that lets you use the Stream Deck without root. With the AppImage, install the rule yourself (see Installation).',
    },
    empty: {
      title: 'No release has been published yet',
      text: 'There is nothing to download at the moment. Releases appear here as soon as they are published on GitHub.',
      releases: 'Releases on GitHub',
      source: 'Build from source',
    },
    noInstallers: 'This release has no installers that this page recognises. See all its files on GitHub.',
    verify: {
      title: 'Verify your download',
      lead: 'Each file’s SHA-256 checksum is shown next to it, as GitHub reports it for the published asset. Compute the checksum of the file you downloaded and compare the two.',
      macLinux: 'macOS and Linux',
      windows: 'Windows (PowerShell)',
      sums: 'The release also carries a SHA256SUMS file with the checksums of all its files.',
      attest: 'Releases have build provenance attestations. With the GitHub CLI you can check that a file was built by this repository’s release workflow:',
    },
    updater: {
      title: 'What the other release files are',
      lead: 'A release contains more than installers. You do not need these files to install DeckPilot.',
      items: [
        { name: '*.sig', text: 'Signatures of the update packages (minisign). The app verifies them against the public key compiled into it before it installs an update.' },
        { name: '*.app.tar.gz', text: 'The macOS update package the app downloads when it updates itself. It is not an installer.' },
        { name: 'prerelease.json, latest.json', text: 'The update manifest the app reads: prerelease.json in a pre-release, latest.json in a stable release.' },
        { name: 'release-meta.json', text: 'A record of the release: tag, version, channel, commit, and how each build was signed.' },
        { name: 'SHA256SUMS', text: 'SHA-256 checksums of all files of the release.' },
      ],
      note: 'Updates are delivered through GitHub Releases only. This website hosts no installers and no update files.',
    },
    requirements: {
      title: 'Before you install',
      items: [
        'A Stream Deck MK.2 is the only supported device. DeckPilot has been tested with real hardware on macOS; Windows and Linux are covered by automated tests only.',
        'There are builds for macOS (Apple Silicon and Intel), Windows x64 and Linux x64. There is no ARM build for Windows or Linux.',
        'On Linux, the tray icon needs AppIndicator (StatusNotifier) support; without it the agent still works.',
      ],
      install: 'Installation guide',
    },
  },

  screenshots: {
    title: 'Screenshots',
    metaTitle: 'DeckPilot screenshots',
    description:
      'A tour of DeckPilot in screenshots: dashboard, layout editor, profiles and projects, action editor, workflow editor, Work Session and settings.',
    lead: 'A tour of the application, one screen at a time.',
    pendingTitle: 'Real screenshots are on their way',
    pendingText:
      'This gallery only ever shows real screenshots of the application. None have been published yet, so the frames below are placeholders that say what each screenshot will show. Nothing here is a mock-up.',
    placeholder: 'Screenshot not available yet',
    placeholderBadge: 'Placeholder',
    willShow: 'Will show',
    items: {
      dashboard: {
        title: 'Dashboard',
        alt: 'The DeckPilot dashboard with the agent status and connected devices',
        text: 'The dashboard: the state of the background agent, the active profile and the connected devices.',
      },
      device: {
        title: 'Layout Designer',
        alt: 'The Layout Designer with the key grid of a Stream Deck MK.2, the action library and the inspector',
        text: 'The Layout Designer: the device canvas, the library of actions and workflows, and the inspector for a key.',
      },
      profiles: {
        title: 'Profiles and projects',
        alt: 'The profiles page and the projects of a profile with their cycle order',
        text: 'Profiles and the projects inside them, with the cycle order and the resources bound on this computer.',
      },
      actionEditor: {
        title: 'Action editor',
        alt: 'The action editor with the parameters of an action',
        text: 'The action editor: name, icon, key label and the parameters of the action type.',
      },
      workflowEditor: {
        title: 'Workflow editor',
        alt: 'The workflow graph editor with nodes, connections and the run panel',
        text: 'The workflow editor: the graph of nodes, the catalog and the history of runs.',
      },
      workSession: {
        title: 'Work Session',
        alt: 'The Work Session page with the current session and the history',
        text: 'Work Session: the current session, its settings and the history of sessions.',
      },
      settings: {
        title: 'Settings and modules',
        alt: 'The settings window with the list of built-in modules',
        text: 'Settings: the built-in modules with their switches, permissions and module settings.',
      },
    },
  },

  docs: {
    title: 'Documentation',
    metaTitle: 'DeckPilot user documentation',
    description:
      'User documentation for DeckPilot: installation, first launch, devices, profiles, layouts, actions, gestures, workflows, variables, the background agent, updates and troubleshooting.',
    lead: 'How to install DeckPilot, connect a Stream Deck, and build your profiles, layouts, actions and workflows.',
    groups: {
      start: 'Getting started',
      design: 'Designing your deck',
      automate: 'Automating',
      run: 'Running DeckPilot',
      help: 'Help',
    },
    sidebarLabel: 'Documentation pages',
    menu: 'Documentation menu',
    onThisPage: 'On this page',
    previous: 'Previous',
    next: 'Next',
    pagination: 'Previous and next page',
    statusTitle: 'About this documentation',
    status:
      'DeckPilot changes quickly. These pages describe the application as its source code stood at the commit below; names in the interface may differ slightly in the version you have installed.',
    uiLanguage: 'The application’s interface is in English; names of screens and buttons are written here as they appear in it.',
    verified: 'Checked against DeckPilot’s source at commit {commit}.',
    source: 'Technical documentation in the repository',
    developersTitle: 'Writing plugins?',
    developers:
      'Plugin development is documented on the Marketplace site: the SDK, the manifest, permissions, testing and publishing.',
    developersLink: 'Open the plugin developer portal',
  },

  notFound: {
    title: 'Page not found',
    description: 'The page you are looking for does not exist on the DeckPilot website.',
    heading: 'This page does not exist',
    text: 'The address may be mistyped, or the page may have moved.',
    home: 'Back to the homepage',
    docs: 'Open the documentation',
  },

  root: {
    title: 'DeckPilot',
  },
};
