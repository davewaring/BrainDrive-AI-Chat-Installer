# Claude Code Context: BrainDrive AI Chat Installer

## Project Overview

This is a chat-based AI installer for BrainDrive. Users visit a web page, chat with Claude, and Claude orchestrates the installation through a local bootstrapper app.

## Architecture

```
Web Chat (React) <--WebSocket--> Backend (Node.js) <--WebSocket--> Bootstrapper (Tauri/Rust)
                                      |
                                 Claude API
```

### Three Components

1. **packages/bootstrapper** - Tauri desktop app (~10MB)
   - Rust backend in `src-tauri/src/`
   - React frontend in `src/`
   - Connects to backend via WebSocket
   - Executes system commands, detects installed software
   - Manages BrainDrive lifecycle (start/stop/restart)

2. **packages/backend** - Node.js server
   - `server.js` - Express + WebSocket server
   - `claude.js` - Claude API integration with tool calling
   - `websocket-hub.js` - Routes messages between browser and bootstrapper
   - `tools.js` - Tool definitions for Claude
   - `session.js` - In-memory session state

3. **packages/web** - Web chat frontend
   - React app served at braindrive.ai/install
   - Chat interface with Claude
   - Shows bootstrapper connection status

## Key Design Decisions

- **Tauri over Electron**: Smaller binary (~10MB vs ~150MB)
- **Claude in cloud, not local**: Best reasoning, tiny download, rapid prompt updates
- **WebSocket for all comms**: Real-time bidirectional messaging
- **Audited tools only**: Claude calls predefined functions, not arbitrary shell commands

## Development Commands

```bash
# From repo root
npm install                      # Install all workspace dependencies

# Backend (needs ANTHROPIC_API_KEY in .env)
cd packages/backend
npm run dev                      # Start with --watch

# Web frontend
cd packages/web
npm run dev                      # Vite dev server on :5174

# Bootstrapper (IMPORTANT: set backend URL for local dev)
cd packages/bootstrapper
BRAINDRIVE_BACKEND_URL=ws://localhost:3000 npm run tauri:dev
cargo check                      # Check Rust compiles (from src-tauri/)
```

**Important:** The bootstrapper defaults to the production backend URL. For local development, you MUST set `BRAINDRIVE_BACKEND_URL=ws://localhost:3000` or the bootstrapper will connect to production instead of your local backend.

## Implementation Status

### Phase 1: Foundation (Complete)
- [x] Tauri project setup with React
- [x] WebSocket client in Rust
- [x] System detection (OS, conda, git, node, ollama)
- [x] Backend with Claude integration
- [x] Web chat UI
- [x] Basic Start/Stop/Restart UI

### Phase 2: System Detection & Tools (Complete)
- [x] Implement actual command execution in Rust
- [x] Port availability checking (IPv4 + IPv6)
- [x] Conda environment creation (isolated to ~/BrainDrive/miniconda3)
- [x] Git clone operations
- [x] Parallel dependency installation (install_all_deps)
- [x] Process lifecycle management (start/stop/restart with PID tracking)

### Phase 3: Ollama + Offline Model (Nearly Complete)
- [x] Ollama detection and installation guidance
- [x] Model pulling with progress streaming
- [x] Full installation flow verified (January 16, 2026)
- [ ] Hardware-based model recommendations

### Phase 4-7: See plan.md in BrainDrive-Planning repo

## Code Patterns

### Adding a new Tauri command

1. Add function in `src-tauri/src/lib.rs`:
```rust
#[tauri::command]
async fn my_command(param: String) -> Result<String, String> {
    // implementation
}
```

2. Register in `invoke_handler`:
```rust
.invoke_handler(tauri::generate_handler![
    // ... existing commands
    my_command,
])
```

3. Call from React:
```typescript
const result = await invoke<string>('my_command', { param: 'value' });
```

### Adding a new Claude tool

1. Add tool definition in `packages/backend/tools.js`
2. Add handler in `packages/backend/claude.js` `_executeTool()` method
3. If tool needs bootstrapper, add message handler in Rust `websocket.rs`

## Environment Variables

### Backend (.env)
```
ANTHROPIC_API_KEY=sk-ant-...   # Required
PORT=3000                       # Optional, default 3000
```

## Testing

Currently manual testing only. To test:
1. Start backend with valid API key
2. Start web frontend
3. Start bootstrapper
4. Chat should connect and Claude should respond

## Related Repos

- **BrainDrive-Core**: The main BrainDrive application being installed
- **BrainDrive-Planning**: Planning docs, feature specs, meeting transcripts
