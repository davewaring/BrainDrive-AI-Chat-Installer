# BrainDrive AI Chat Installer

A chat-based AI installer for BrainDrive, powered by Claude.

## Architecture

```
┌─────────────────────────────────────────┐
│   braindrive.ai/install (Web Chat UI)   │
│   - Full Claude streaming responses     │
│   - Download prompts                    │
│   - Progress indicators                 │
└────────────────┬────────────────────────┘
                 │ WebSocket
                 ▼
┌─────────────────────────────────────────┐
│   Backend API (Node.js)                 │
│   - Claude API integration              │
│   - WebSocket hub routing               │
│   - Session management                  │
└────────────────┬────────────────────────┘
                 │ WebSocket
                 ▼
┌─────────────────────────────────────────┐
│   Tauri Bootstrapper (~10MB .app/.exe)  │
│   - Minimal UI (status, controls)       │
│   - System detection                    │
│   - Command execution                   │
│   - Start/Stop/Restart controls         │
└─────────────────────────────────────────┘
```

## Packages

- **packages/bootstrapper** - Tauri desktop app (Rust + React)
- **packages/backend** - Node.js WebSocket server with Claude integration
- **packages/web** - Web chat frontend (React)

## Development

### Prerequisites

- Node.js 18+
- Rust toolchain
- Tauri CLI (`cargo install tauri-cli`)

### Setup

```bash
# Install dependencies
npm install

# Start backend (requires ANTHROPIC_API_KEY in packages/backend/.env)
cd packages/backend
cp .env.example .env
npm run dev

# Start web frontend (in another terminal)
cd packages/web
npm run dev

# Start bootstrapper (in another terminal)
cd packages/bootstrapper
npm run tauri:dev
```

## License

MIT
