# Claude Code Context: BrainDrive AI Chat Installer

## Current Status

| Field | Value |
|-------|-------|
| **Version** | v0.1.4-alpha |
| **Phase** | 3 (Ollama + Offline Model) - nearly complete |
| **Branch** | `feature-audited-helpers` |
| **Last Verified** | January 16, 2026 - full e2e install working |

### Planning Docs

Full roadmap, specs, and transcripts are in the planning repo:
- **Main plan:** `~/BrainDrive-Planning/plans/active/ai-installer/plan.md` (comprehensive, 1000+ lines)
- **Feature spec:** `~/BrainDrive-Planning/plans/active/ai-installer/feature-spec.md`
- **Deployment:** `~/BrainDrive-Planning/plans/active/ai-installer/deployment-plan.md`

### What's Done
- Isolated Miniconda installation (`~/BrainDrive/miniconda3`)
- Parallel dependency installation (`install_all_deps`)
- Ollama detection + progress streaming
- Process cleanup on app close
- Full installation flow: detect → install_conda → clone → create_env → install_deps → setup_env → start

### What's Next (Phase 3 Remaining)
- [ ] Hardware-based Qwen model recommendations
- [ ] Model checksum verification after download
- [ ] Claude outage contingency (fallback state machine)
- [ ] Update helpers: `check_for_updates`, `update_braindrive_services`, `update_conda_env`

### Future Phases
- **Phase 4:** Default user creation, Check for Updates UI
- **Phase 5:** Reliability (retry flows, log viewer, macOS notarization)
- **Phase 6:** Windows build + signing + QA
- **Phase 7:** Launch readiness

---

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

See "Current Status" section at top for latest. Full details in `~/BrainDrive-Planning/plans/active/ai-installer/plan.md`.

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

- **BrainDrive-Planning** (`~/BrainDrive-Planning`): Planning docs, feature specs, meeting transcripts
  - AI Installer plan: `plans/active/ai-installer/plan.md`
- **BrainDrive** (`~/BrainDrive`): The main BrainDrive application being installed
