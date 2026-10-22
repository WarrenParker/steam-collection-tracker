# How the app is built

A guide to the code, the data folder, and how versions get built and released.

## Stack

- **Engine:** Rust, in `src-tauri/`. Talks to Steam, keeps settings, runs scans and tests, and updates the app.
- **Interface:** Svelte 5 and TypeScript, in `src/`. Built with Vite into static files that are embedded in the exe.
- **Shell:** [Tauri 2](https://tauri.app). It opens a native window and draws the interface with WebView2, the Edge engine built into Windows 10 and 11. The result is one exe of about 7 MB.

## Repository layout

```text
.github/workflows/build.yml   Builds, tests and releases on every push to main
CHANGELOG.md                  Every version's changes; release notes come from here
docs/                         Roadmap, idea board, architecture, Steam data notes
index.html, src/              The interface
  App.svelte                  Top bar, setup vs overview, settings drawer
  app.css                     Design tokens and themes (greyscale, dark, light)
  lib/api.ts                  Typed calls into the engine
  lib/*.svelte                Screens and components
src-tauri/                    The engine
  tauri.conf.json             App name, bundle settings, content security policy
  capabilities/default.json   What the interface window is allowed to do
  icons/                      App icon (generated, see below)
  src/lib.rs                  Startup: tidy-up, settings, main window, update check
  src/commands.rs             Everything the interface can ask the engine to do
  src/steam/                  Paced Steam client, response parsing, inventory tally
  src/session.rs              Steam sign-in, renewal and sign-out
  src/scan.rs                 The scan behind the overview
  src/diagnostics.rs          The connection test and its report
  src/updater.rs              Update checks, version history, installing any version
  src/housekeeping.rs         Automatic tidy-up and folder size
  src/settings.rs             Settings file; the API key is encrypted with secret.rs
  src/secret.rs               Windows DPAPI encryption
  src/paths.rs                Where the data folder is
  src/profile_input.rs        Understands pasted profile links, names and IDs
```

## The data folder

Everything the app writes goes in a `data` folder next to the exe. If that folder can't be written to, for example under Program Files, it uses `%LOCALAPPDATA%\SteamCollectionTracker` instead.

```text
data/
  settings.json        Theme, update choice, profile, encrypted API key
  cache/last_scan.json The most recent scan
  reports/             Connection test reports (the newest 5 are kept)
  update/              Downloads in progress (cleared at every launch)
  webview/             WebView2 profile, including Steam's sign-in cookies
```

At every launch, `housekeeping.rs` clears these:

- leftover update downloads
- exe files left over from an update swap
- old reports
- WebView caches

The sign-in cookies are kept.

## Talking to Steam

- All requests go through `SteamClient` in `src/steam/mod.rs`. Each kind of request has a minimum gap:

  | Kind of request | Minimum gap |
  |---|---|
  | Web API | 0.3 s |
  | Community pages | 3.5 s |
  | Inventory | 4 s |
  | Market | 4 s |
  | Store | 1 s |

- When Steam refuses a request (HTTP 429), the client stops sending that kind for 30 minutes instead of retrying. Only Web API calls are retried. The details are in [STEAM-DATA.md](STEAM-DATA.md).
- **Sign-in** opens Steam's own sign-in page in a second window, which shares the WebView2 profile. When the `steamLoginSecure` cookie appears, the app reads it through Tauri's cookie API and closes the window.
- **Renewal.** When the cookie is about to expire, the app loads a Steam page in a hidden window, and Steam renews the session itself.
- **Sign-out** loads Steam in a hidden window and posts to `/login/logout/`, so Steam revokes the session on its side too.
- **Web API token.** Calls made as the signed-in user use a Web API token from `pointssummary/ajaxgetasyncconfig`. If that's unavailable, they use the token inside the sign-in cookie.

## Versions, builds and updates

- **Semantic versioning** (MAJOR.MINOR.PATCH). The version is set in `src-tauri/Cargo.toml` and `package.json`, and every version has a section in `CHANGELOG.md`.
- **Every push to `main`** runs `.github/workflows/build.yml` on a Windows runner: interface build, `cargo test`, `tauri build`, then packaging.
  - The build posts a short report (test results, warnings, size) as a check annotation, or the tail of the log if it fails. These can be read through the GitHub API.
  - Pushes that only change `docs/` or `README.md` don't trigger a build.
- **When the version is new,** the workflow publishes a GitHub release tagged `vX.Y.Z`. Its notes are that version's changelog section, and it carries two files: `SteamCollectionTracker.exe` and `SteamCollectionTracker.exe.sha256`. The build stops if the changelog has no entry for the version.
- **The app** checks the latest release when it starts. The repository and build commit are baked in at build time through `UPDATE_REPO` and `GITHUB_SHA`. When it finds a newer version, the app:
  1. downloads it
  2. checks the SHA-256
  3. swaps the exe with `self_replace`
  4. offers "Restart now"

  Settings lists every release and can install any of them. Installing an older version turns automatic updates off.

## Security and privacy

- **API key:** encrypted with Windows DPAPI for the current user. It never reaches the interface; only its last four characters are shown.
- **Steam sign-in:** happens on Steam's own page. The app never reads or stores the password. The cookies live in the WebView2 profile, which Windows encrypts.
- **Reports:** every secret (API key, cookies, tokens) is blanked out before a connection test report is written.
- **External links:** the interface can only open links to Steam and GitHub, and only in the user's own browser.
- **Content security policy:** the interface loads nothing from the internet except images.

## Conventions

- **Messages are plain language.** They say what happened and what to do next, and never show URLs, codes or secrets unless that helps.
- **Commits** describe the change in plain words. Each release is one version bump with a matching changelog entry.
- **Icons** in `src-tauri/icons/` are generated from a script (a stack of cards with a foil front). Regenerate all sizes together if the icon changes.
