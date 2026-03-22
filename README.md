# Postiz Setup Wizard

[Postiz](https://postiz.com) is an open-source social media scheduling and analytics platform — a self-hostable alternative to Buffer, Hootsuite, and Later. It lets you compose posts, schedule them across multiple social platforms, and track engagement from a single dashboard.

**This wizard lets you self-host Postiz on your Windows PC in minutes, with zero command line knowledge.** It handles everything — installing Docker, configuring services, setting up HTTPS, and connecting your social accounts — through a simple step-by-step interface.

[![Download](https://img.shields.io/github/v/release/SamWylde/postiz-setup-wizard?label=Download&style=for-the-badge)](https://github.com/SamWylde/postiz-setup-wizard/releases/latest)

## Features

### Guided Setup (6 steps, ~10 minutes)
1. **Prepare Computer** — Automatically detects and installs prerequisites (WSL2, Docker Desktop, tunnel providers)
2. **Install Postiz** — Choose install location, auto-generates secrets, pulls Docker images, starts all services
3. **Create Account** — Waits for Postiz to come online, then walks you through first account creation
4. **Create Web Link** — Sets up a public HTTPS URL via your choice of tunnel provider (required for OAuth callbacks). Supports **Cloudflare**, **ngrok**, **zrok**, and **Pinggy**
5. **Connect Platforms** — Step-by-step guides for each social platform, with pre-filled callback URLs you can copy with one click
6. **Verify & Finish** — Runs health checks and confirms everything is working

### After Setup
- **System tray app** — Keeps your tunnel running and services monitored in the background
- **Live dashboard** — Container health, tunnel status, and overall system health at a glance
- **Auto-updates** — Checks for new versions and installs them with one click
- **Docker log viewer** — View container logs without opening a terminal
- **Diagnostics export** — One-click export for troubleshooting
- **Recovery center** — Detects problems on startup and offers guided repair

### Supported Social Platforms

Facebook · Instagram · LinkedIn · X (Twitter) · Reddit · Threads · YouTube · TikTok · Pinterest · Discord · Slack · Mastodon · Bluesky · Dribbble

## Requirements

- Windows 10 or later
- ~3 GB free disk space, ~2 GB available RAM

That's it. The wizard detects and installs everything else automatically.

---

## Development

### Getting Started

You'll need [Node.js](https://nodejs.org/) (v20+), [Rust](https://rustup.rs/), and the Visual Studio "Desktop development with C++" workload.

```bash
npm install
npm run tauri dev
```

### Building

```bash
npm run tauri build
```

Build from a **Developer Command Prompt** (not Git Bash) to avoid MSVC linker conflicts.

### Project Structure

```
src/                    # React frontend
  screens/              # Wizard step screens + dashboard + recovery
  components/           # UI components + social provider registry
  store/                # Zustand state management
  lib/                  # Tauri command wrappers

src-tauri/              # Rust backend
  src/commands/         # Tauri commands (bootstrap, docker, tunnel, env, updater)
  src/templates/        # Embedded docker-compose.yml + Temporal config
  src/state.rs          # Shared application state
```

### Tech Stack

- **Desktop:** Tauri 2 (Rust + system WebView)
- **Frontend:** React 19, TypeScript, Tailwind CSS v4, Zustand
- **Infrastructure:** Docker Compose, cloudflared / ngrok / zrok / Pinggy

### Releasing

Versions are defined in three files that **must** stay in sync:
- `package.json` → `version`
- `src-tauri/Cargo.toml` → `version`
- `src-tauri/tauri.conf.json` → `version`

To release, update all three, commit, then push a tag:

```bash
git tag v0.2.0
git push origin v0.2.0
```

GitHub Actions will build the installer, sign it, create a GitHub Release with the `.exe`, `.msi`, and updater manifest (`latest.json`). Existing installations will detect the new version automatically.

## License

[MIT](LICENSE)
