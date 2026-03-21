# Postiz Setup Wizard

[Postiz](https://postiz.com) is an open-source social media scheduling and analytics platform — a self-hostable alternative to Buffer, Hootsuite, and Later. It lets you compose posts, schedule them across multiple social platforms, and track engagement from a single dashboard.

This wizard is a Windows desktop app that automates self-hosting Postiz, turning hours of terminal work into a guided point-and-click experience. No command line knowledge required.

## Download

Grab the latest installer from the [Releases page](https://github.com/SamWylde/postiz-setup-wizard/releases/latest).

## What it does

Self-hosting Postiz normally requires Docker setup, configuring 50+ environment variables, HTTPS tunnel setup, and OAuth app registration across multiple developer portals. This wizard handles all of it through a 6-step guided flow:

1. **Prepare Computer** — Detects and installs prerequisites (WSL2, Docker Desktop, cloudflared)
2. **Install Postiz** — Picks install location, generates secrets, pulls Docker images, starts containers
3. **Create Account** — Verifies Postiz is running and guides you through account creation
4. **Create Web Link** — Sets up a Cloudflare Quick Tunnel for HTTPS access (needed for OAuth)
5. **Connect Platforms** — Step-by-step sub-wizards for 15 social platforms with copyable callback URLs
6. **Verify & Finish** — Health checks, summary, and system tray minimization

After setup, the app runs in the system tray to maintain the tunnel and monitor services. Includes a live status dashboard, automatic update checks, Docker log viewer, diagnostics export, and a recovery center for troubleshooting.

## Supported Platforms

Facebook, Instagram, LinkedIn, X (Twitter), Reddit, Threads, YouTube, TikTok, Pinterest, Discord, Slack, Mastodon, Bluesky, Dribbble

## Tech Stack

- **Desktop framework:** Tauri 2 (Rust backend + system WebView)
- **Frontend:** React 19, TypeScript, Tailwind CSS v4, Zustand
- **Infrastructure:** Docker Compose, cloudflared Quick Tunnels

## Prerequisites

- Windows 10 or later
- ~3 GB free disk space
- ~2 GB available RAM

The wizard will detect and offer to install any missing dependencies (WSL2, Docker Desktop, cloudflared).

## Development

```bash
# Install frontend dependencies
npm install

# Run in development mode
npm run tauri dev

# Build for production
npm run tauri build
```

### Project Structure

```
src/                    # React frontend
  screens/              # Wizard step screens + dashboard/recovery
  components/           # UI components + provider registry
  store/                # Zustand state management
  lib/                  # Tauri command wrappers

src-tauri/              # Rust backend
  src/commands/         # Tauri commands (bootstrap, docker, tunnel, env, etc.)
  src/templates/        # Embedded docker-compose.yml + Temporal config
  src/state.rs          # Shared application state
```

## License

MIT
