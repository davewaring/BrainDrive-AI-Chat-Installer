import Anthropic from '@anthropic-ai/sdk';
import { TOOLS } from './tools.js';

const SYSTEM_PROMPT = `You are the BrainDrive Installation Assistant, a friendly and knowledgeable guide helping users install BrainDrive on their computer.

## Your Personality
- Warm, approachable, and patient
- Use simple language - avoid technical jargon unless necessary
- Be proactive - guide users through the process without requiring them to ask what to do next
- Celebrate small wins along the way
- If something goes wrong, reassure the user and help them recover

## Your Capabilities
You can interact with the user's computer through a local bootstrapper app. You have access to tools that let you:
- Detect system information (OS, installed software, hardware)
- Clone the BrainDrive repository from GitHub
- Create and manage conda environments
- Install Python and npm dependencies
- Configure environment files
- Start, stop, and restart BrainDrive services
- Check port availability

## CRITICAL RULES - READ CAREFULLY

1. **NEVER claim BrainDrive is installed or ready without FIRST calling detect_system**
   - You do NOT know the current system state until you check
   - ALWAYS call detect_system BEFORE making ANY claims about what's installed
   - Do NOT hallucinate or assume system state - VERIFY with tools

2. **NEVER skip straight to "you're all set!" without going through the installation flow**
   - Even if user says "let's do it" or seems eager, you MUST check first
   - Each step must be verified, not assumed

3. **When user wants to start installation, your FIRST action MUST be calling detect_system**
   - This is non-negotiable - no exceptions
   - Only after seeing the tool results can you describe what's installed

## Installation Flow
Follow this sequence once the bootstrapper is connected:

1. **Detect System** - Use \`detect_system\` to check what's installed
   - Returns: conda_installed, braindrive_env_ready, braindrive_exists, ollama_installed
   - Note: OS, architecture, available memory

2. **Install Conda if needed** - If conda_installed is false, use \`install_conda\`
   - Automatically downloads and installs Miniconda to ~/BrainDrive/miniconda3
   - This is isolated from any existing system conda installation
   - No terminal or sudo required - fully automated!
   - Shows download progress in the UI
   - NOTE: Git and Node.js are included in the conda env, NOT installed separately

3. **Clone Repository** - Use \`clone_repo\` to download BrainDrive
   - Clones to ~/BrainDrive by default
   - Uses shallow clone for speed
   - Uses git from conda env (installed in step 5)

4. **Create Conda Environment** - Use \`create_conda_env\`
   - Creates "BrainDriveDev" environment
   - Includes Python 3.11, Node.js, AND Git from conda-forge
   - If env exists but is missing nodejs, run with force_recreate=true

5. **Install All Dependencies** - Use \`install_all_deps\`
   - Installs both backend and frontend dependencies IN PARALLEL
   - This is faster than installing them separately (~1-1.5 min saved)
   - Backend: Python packages from requirements.txt in conda env
   - Frontend: npm install in the frontend directory (uses npm from conda)

6. **Setup Environment File** - Use \`setup_env_file\`
   - Copies .env-dev to .env

7. **Start BrainDrive** - Use \`start_braindrive\`
   - Starts backend on port 8005
   - Starts frontend on port 5173

8. **Offer Offline Model (Optional)** - If the user wants offline chat, set up Ollama
   - Use \`install_ollama\` to detect and start Ollama if installed
   - If not installed, share the download link + short OS steps
   - Ask the user to confirm when finished, then re-run \`install_ollama\` to detect/start
   - Keep the user in the chat window the whole time

9. **Celebrate!** - Open BrainDrive in browser

## Pre-Bootstrapper Flow
If the bootstrapper is NOT connected:
1. Greet the user warmly
2. Explain BrainDrive briefly (AI-powered personal productivity platform)
3. Explain you need them to download a small helper app
4. They'll see a download button - guide them to click it
5. Once downloaded, they should open it
6. Wait for connection, then proceed with installation

## Important Guidelines
- Always use \`check_connection\` first if unsure about bootstrapper status
- Explain what each step does BEFORE running the tool
- Wait for confirmation on major steps (cloning, creating env)
- Ask for confirmation before starting BrainDrive services
- If a step fails, explain what went wrong simply and offer solutions
- Long operations (clone, npm install) may take a minute - reassure the user
- If something already exists (repo, env), that's fine - move to next step
- **NEVER call the same tool twice in a row** - if a tool succeeds, move on; if it fails, diagnose first
- **start_braindrive is idempotent** - it will return success if already running, no need to retry
- Keep the user in the chat window; provide links and clear steps instead of sending them elsewhere
- You may re-run \`install_ollama\` after the user confirms they finished installing it

## Handling Existing Installations (IMPORTANT)
When detect_system shows BrainDrive is already installed (conda_installed=true, braindrive_env_ready=true, braindrive_exists=true):

1. **Tell the user what you found** - Be specific: "I checked your system and found BrainDrive is already installed!"
2. **Offer clear options:**
   - "Would you like me to start BrainDrive?" (if not running)
   - "Would you like to reinstall from scratch?" (fresh install)
   - "BrainDrive is already running at [URL]!" (if already running)
3. **Do NOT assume** - Wait for the user to choose before taking action
4. **Do NOT immediately say "you're all set!"** - Explain what you found first

## Tool Behavior Notes
- \`install_conda\`: Downloads and installs Miniconda to ~/BrainDrive/miniconda3 (isolated installation). Returns success with \`already_installed: true\` if the isolated conda is already present. Git and Node are installed via conda env, not separately.
- \`start_braindrive\`: Automatically finds available ports if defaults are taken. Returns success if already running. Requires explicit user confirmation; include \`confirmed: true\` only after the user approves.
- \`clone_repo\`: Returns success with \`already_exists: true\` if repo exists. Uses git from the conda environment.
- \`create_conda_env\`: Creates env with Python 3.11, nodejs, and git from conda-forge. Returns success with \`already_exists: true\` if env exists. Use force_recreate=true if npm/node is missing.
- \`install_all_deps\`: Runs backend and frontend dependency installation IN PARALLEL. Returns detailed results for both. Preferred over separate install_backend_deps + install_frontend_deps calls. Uses npm from the conda environment.
- \`setup_env_file\`: Returns success with \`already_exists: true\` if .env exists.
- \`install_ollama\`: Starts Ollama if installed. If missing, returns \`download_url\` and \`instructions\` for manual install. After the user finishes, call \`install_ollama\` again to detect and start it.
- \`pull_ollama_model\`: Requires explicit user confirmation; include \`confirmed: true\` only after the user approves.

## Error Recovery
- If conda not installed: Use \`install_conda\` to automatically install Miniconda to ~/BrainDrive/miniconda3 (no user action needed!)
- If npm/node not found after create_conda_env: The env may have been created without nodejs. Use \`create_conda_env\` with force_recreate=true to recreate it properly.
- If start_braindrive fails: Check the error message - it includes log paths for debugging
- If clone fails: Check internet connection, try again
- If install_conda fails: Check internet connection; may need to retry or ask user to install manually from https://docs.conda.io/en/latest/miniconda.html

## Conversation Style
- Short paragraphs and bullet points
- Occasional encouragement during waits: "This might take a minute - BrainDrive has some powerful features!"
- Be honest about issues
- Celebrate completion enthusiastically!
- Never download Ollama models without explicit user approval; always propose a model first and ask to proceed.

Remember: Your goal is to make installation feel effortless. The user should feel guided every step of the way.`;

export class ClaudeClient {
  constructor(session, hub) {
    this.session = session;
    this.hub = hub;
    this.client = new Anthropic();
    this.model = 'claude-sonnet-4-20250514';
    this.isProcessing = false;
    this.messageQueue = [];
  }

  _isCoreInstallReady() {
    if (this.session.installState === 'completed') {
      return true;
    }
    const info = this.session.systemInfo;
    return Boolean(
      info &&
      info.conda_installed &&
      info.braindrive_env_ready &&
      info.braindrive_exists
    );
  }

  _requireCoreInstallReady(actionLabel) {
    if (this._isCoreInstallReady()) {
      return null;
    }
    return {
      error: `BrainDrive is not installed yet. Please complete the core installation before ${actionLabel}.`,
    };
  }

  async processMessage(userMessage) {
    // Queue messages to prevent concurrent processing
    this.messageQueue.push(userMessage);

    if (this.isProcessing) {
      return;
    }

    this.isProcessing = true;

    while (this.messageQueue.length > 0) {
      const message = this.messageQueue.shift();
      await this._processMessageInternal(message);
    }

    this.isProcessing = false;
  }

  async _processMessageInternal(userMessage) {
    // Add user message to history
    this.session.addMessage('user', userMessage);

    try {
      // Send typing indicator
      this.hub.sendToBrowser({ type: 'ai_typing', typing: true });

      // Call Claude API with streaming
      await this._streamResponse();

    } catch (error) {
      console.error('Claude API error:', error);
      this.hub.sendToBrowser({
        type: 'error',
        message: `AI error: ${error.message}`,
      });
    } finally {
      this.hub.sendToBrowser({ type: 'ai_typing', typing: false });
    }
  }

  async _streamResponse() {
    const stream = this.client.messages.stream({
      model: this.model,
      max_tokens: 4096,
      system: SYSTEM_PROMPT,
      tools: TOOLS,
      messages: this.session.getConversationHistory(),
    });

    let streamingStarted = false;
    let fullText = '';

    // Stream text to browser as it arrives
    stream.on('text', (text) => {
      if (!streamingStarted) {
        streamingStarted = true;
        this.hub.sendToBrowser({ type: 'ai_message_start' });
      }
      fullText += text;
      this.hub.sendToBrowser({
        type: 'ai_message_delta',
        content: text,
      });
    });

    // Wait for stream to complete
    const finalMessage = await stream.finalMessage();

    // Signal end of text if we streamed any
    if (streamingStarted) {
      this.hub.sendToBrowser({ type: 'ai_message_end' });
    }

    // Add assistant response to history
    this.session.addMessage('assistant', finalMessage.content);

    // Check for tool calls in the final message
    const toolCalls = finalMessage.content.filter(block => block.type === 'tool_use');
    if (toolCalls.length > 0) {
      await this._executeToolsAndContinue(toolCalls);
    }
  }

  async _executeToolsAndContinue(toolCalls) {
    const toolResults = [];

    for (const tool of toolCalls) {
      // Notify browser about tool execution
      this.hub.sendToBrowser({
        type: 'tool_executing',
        tool: tool.name,
        input: tool.input,
      });

      const result = await this._executeTool(tool);
      toolResults.push({
        type: 'tool_result',
        tool_use_id: tool.id,
        content: JSON.stringify(result),
      });
    }

    // Add tool results to history
    this.session.addMessage('user', toolResults);

    // Continue conversation with streaming
    await this._streamResponse();
  }

  async _executeTool(tool) {
    const { name, input } = tool;

    console.log(`Executing tool: ${name}`, input);

    try {
      switch (name) {
        case 'check_connection':
          return {
            connected: this.hub.isBootstrapperConnected(),
          };

        case 'detect_system':
          if (!this.hub.isBootstrapperConnected()) {
            return { error: 'Bootstrapper not connected. Please ask the user to download and open the bootstrapper app.' };
          }
          const systemInfo = await this.hub.callBootstrapperTool('detect_system', {}, 30000);
          if (systemInfo.success && systemInfo.data) {
            this.session.setSystemInfo(systemInfo.data);
            if (
              systemInfo.data.conda_installed &&
              systemInfo.data.braindrive_env_ready &&
              systemInfo.data.braindrive_exists
            ) {
              this.session.setInstallState('completed');
            }
            return systemInfo.data;
          }
          return { error: systemInfo.error || 'Failed to detect system' };

        case 'install_conda': {
          if (!this.hub.isBootstrapperConnected()) {
            return { error: 'Bootstrapper not connected' };
          }
          // Miniconda installation can take several minutes for download + install
          const installCondaResult = await this.hub.callBootstrapperTool('install_conda', {}, 600000);
          return installCondaResult.data || installCondaResult;
        }

        case 'install_git': {
          if (!this.hub.isBootstrapperConnected()) {
            return { error: 'Bootstrapper not connected' };
          }
          // Git installation can take several minutes (especially on macOS where user needs to click Install)
          const installGitResult = await this.hub.callBootstrapperTool('install_git', {}, 660000);
          return installGitResult.data || installGitResult;
        }

        case 'install_conda_env': {
          if (!this.hub.isBootstrapperConnected()) {
            return { error: 'Bootstrapper not connected' };
          }
          const envName = (input.env_name || '').trim();
          if (!/^[A-Za-z0-9_-]+$/.test(envName)) {
            return { error: 'Environment name may only include letters, numbers, "-", and "_"' };
          }

          const payload = {
            env_name: envName,
          };

          if (input.repo_path) {
            payload.repo_path = input.repo_path;
          }

          if (input.environment_file) {
            payload.environment_file = input.environment_file;
          }

          const condaResult = await this.hub.callBootstrapperTool('install_conda_env', payload, 300000);
          return condaResult.data || condaResult;
        }

        case 'install_ollama': {
          if (!this.hub.isBootstrapperConnected()) {
            return { error: 'Bootstrapper not connected' };
          }
          const guard = this._requireCoreInstallReady('setting up Ollama');
          if (guard) {
            return guard;
          }
          const ollamaResult = await this.hub.callBootstrapperTool('install_ollama', {}, 600000);
          const payload = ollamaResult.data || ollamaResult;
          if (payload && payload.needs_manual_install) {
            this.hub.sendToBrowser({
              type: 'ollama_install_required',
              download_url: payload.download_url,
              instructions: payload.instructions,
              message: payload.message,
            });
          } else if (payload && payload.success) {
            this.hub.sendToBrowser({ type: 'ollama_install_cleared' });
          }
          return payload;
        }

        case 'start_ollama': {
          if (!this.hub.isBootstrapperConnected()) {
            return { error: 'Bootstrapper not connected' };
          }
          const guard = this._requireCoreInstallReady('starting Ollama');
          if (guard) {
            return guard;
          }
          const startResult = await this.hub.callBootstrapperTool('start_ollama', {}, 60000);
          const payload = startResult.data || startResult;
          if (payload && payload.success) {
            this.hub.sendToBrowser({ type: 'ollama_install_cleared' });
          }
          return payload;
        }

        case 'pull_ollama_model': {
          if (!this.hub.isBootstrapperConnected()) {
            return { error: 'Bootstrapper not connected' };
          }
          const guard = this._requireCoreInstallReady('downloading models');
          if (guard) {
            return guard;
          }
          if (!input.confirmed) {
            return { error: 'User confirmation required before downloading a model.' };
          }
          const model = (input.model || '').trim();
          if (!/^[A-Za-z0-9._:+/-]+$/.test(model)) {
            return { error: 'Model names may only include letters, numbers, ".", "_", "-", "/", and ":"' };
          }

          const payload = {
            model,
            force: Boolean(input.force),
          };

          if (input.registry) {
            payload.registry = input.registry;
          }

          const modelResult = await this.hub.callBootstrapperTool('pull_ollama_model', payload, 600000);
          return modelResult.data || modelResult;
        }

        case 'check_port_available':
          if (!this.hub.isBootstrapperConnected()) {
            return { error: 'Bootstrapper not connected' };
          }
          const portResult = await this.hub.callBootstrapperTool('check_port', {
            port: input.port,
          }, 10000);
          return portResult.data || portResult;

        case 'clone_repo': {
          if (!this.hub.isBootstrapperConnected()) {
            return { error: 'Bootstrapper not connected' };
          }
          const payload = {};
          if (input.repo_url) {
            // Basic URL validation
            if (!input.repo_url.startsWith('https://') && !input.repo_url.startsWith('git@')) {
              return { error: 'Repository URL must start with https:// or git@' };
            }
            payload.repo_url = input.repo_url;
          }
          if (input.target_path) {
            payload.target_path = input.target_path;
          }
          const cloneResult = await this.hub.callBootstrapperTool('clone_repo', payload, 300000);
          return cloneResult.data || cloneResult;
        }

        case 'create_conda_env': {
          if (!this.hub.isBootstrapperConnected()) {
            return { error: 'Bootstrapper not connected' };
          }
          const payload = {};
          if (input.env_name) {
            if (!/^[A-Za-z0-9_-]+$/.test(input.env_name)) {
              return { error: 'Environment name may only include letters, numbers, "-", and "_"' };
            }
            payload.env_name = input.env_name;
          }
          const createEnvResult = await this.hub.callBootstrapperTool('create_conda_env', payload, 300000);
          return createEnvResult.data || createEnvResult;
        }

        case 'install_backend_deps': {
          if (!this.hub.isBootstrapperConnected()) {
            return { error: 'Bootstrapper not connected' };
          }
          const payload = {};
          if (input.env_name) {
            if (!/^[A-Za-z0-9_-]+$/.test(input.env_name)) {
              return { error: 'Environment name may only include letters, numbers, "-", and "_"' };
            }
            payload.env_name = input.env_name;
          }
          if (input.repo_path) {
            payload.repo_path = input.repo_path;
          }
          const backendDepsResult = await this.hub.callBootstrapperTool('install_backend_deps', payload, 600000);
          return backendDepsResult.data || backendDepsResult;
        }

        case 'install_frontend_deps': {
          if (!this.hub.isBootstrapperConnected()) {
            return { error: 'Bootstrapper not connected' };
          }
          const payload = {};
          if (input.env_name) {
            if (!/^[A-Za-z0-9_-]+$/.test(input.env_name)) {
              return { error: 'Environment name may only include letters, numbers, "-", and "_"' };
            }
            payload.env_name = input.env_name;
          }
          if (input.repo_path) {
            payload.repo_path = input.repo_path;
          }
          const frontendDepsResult = await this.hub.callBootstrapperTool('install_frontend_deps', payload, 300000);
          return frontendDepsResult.data || frontendDepsResult;
        }

        case 'install_all_deps': {
          if (!this.hub.isBootstrapperConnected()) {
            return { error: 'Bootstrapper not connected' };
          }
          const payload = {};
          if (input.env_name) {
            if (!/^[A-Za-z0-9_-]+$/.test(input.env_name)) {
              return { error: 'Environment name may only include letters, numbers, "-", and "_"' };
            }
            payload.env_name = input.env_name;
          }
          if (input.repo_path) {
            payload.repo_path = input.repo_path;
          }
          // Longer timeout since this runs both in parallel but we wait for both to complete
          const allDepsResult = await this.hub.callBootstrapperTool('install_all_deps', payload, 600000);
          return allDepsResult.data || allDepsResult;
        }

        case 'setup_env_file': {
          if (!this.hub.isBootstrapperConnected()) {
            return { error: 'Bootstrapper not connected' };
          }
          const payload = {};
          if (input.repo_path) {
            payload.repo_path = input.repo_path;
          }
          const envFileResult = await this.hub.callBootstrapperTool('setup_env_file', payload, 10000);
          return envFileResult.data || envFileResult;
        }

        case 'start_braindrive':
          if (!this.hub.isBootstrapperConnected()) {
            return { error: 'Bootstrapper not connected' };
          }
          if (!input.confirmed) {
            return { error: 'User confirmation required before starting BrainDrive.' };
          }
          const startResult = await this.hub.callBootstrapperTool('start_braindrive', {
            frontend_port: input.frontend_port || 5173,
            backend_port: input.backend_port || 8005,
          }, 60000);
          if (startResult.success) {
            this.session.setBraindriveStatus('running');
            this.session.setInstallState('completed');
          }
          return startResult.data || startResult;

        case 'stop_braindrive':
          if (!this.hub.isBootstrapperConnected()) {
            return { error: 'Bootstrapper not connected' };
          }
          const stopResult = await this.hub.callBootstrapperTool('stop_braindrive', {}, 30000);
          if (stopResult.success) {
            this.session.setBraindriveStatus('stopped');
          }
          return stopResult.data || stopResult;

        case 'restart_braindrive':
          if (!this.hub.isBootstrapperConnected()) {
            return { error: 'Bootstrapper not connected' };
          }
          const restartResult = await this.hub.callBootstrapperTool('restart_braindrive', {}, 60000);
          return restartResult.data || restartResult;

        default:
          return { error: `Unknown tool: ${name}` };
      }
    } catch (error) {
      console.error(`Tool ${name} failed:`, error);
      return { error: error.message };
    }
  }
}
