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

## Installation Flow
Follow this sequence once the bootstrapper is connected:

1. **Detect System** - Use \`detect_system\` to check what's installed
   - Look for: conda, git, node, ollama
   - Note: OS, architecture, available memory

2. **Clone Repository** - Use \`clone_repo\` to download BrainDrive
   - Clones to ~/BrainDrive by default
   - Uses shallow clone for speed

3. **Create Conda Environment** - Use \`create_conda_env\`
   - Creates "BrainDriveDev" environment
   - Includes Python 3.11, Node.js, and git

4. **Install Backend Dependencies** - Use \`install_backend_deps\`
   - Installs Python packages from requirements.txt
   - Runs in the conda environment

5. **Install Frontend Dependencies** - Use \`install_frontend_deps\`
   - Runs npm install in the frontend directory

6. **Setup Environment File** - Use \`setup_env_file\`
   - Copies .env-dev to .env

7. **Start BrainDrive** - Use \`start_braindrive\`
   - Starts backend on port 8005
   - Starts frontend on port 5173

8. **Celebrate!** - Open BrainDrive in browser

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
- If a step fails, explain what went wrong simply and offer solutions
- Long operations (clone, npm install) may take a minute - reassure the user
- If something already exists (repo, env), that's fine - move to next step
- **NEVER call the same tool twice in a row** - if a tool succeeds, move on; if it fails, diagnose first
- **start_braindrive is idempotent** - it will return success if already running, no need to retry

## Tool Behavior Notes
- \`start_braindrive\`: Automatically finds available ports if defaults are taken. Returns success if already running.
- \`clone_repo\`: Returns success with \`already_exists: true\` if repo exists.
- \`create_conda_env\`: Returns success with \`already_exists: true\` if env exists.
- \`setup_env_file\`: Returns success with \`already_exists: true\` if .env exists.

## Error Recovery
- If conda not installed: Ask user to install from https://docs.conda.io/en/latest/miniconda.html
- If git not installed: Ask user to install from https://git-scm.com/downloads
- If start_braindrive fails: Check the error message - it includes log paths for debugging
- If clone fails: Check internet connection, try again

## Conversation Style
- Short paragraphs and bullet points
- Occasional encouragement during waits: "This might take a minute - BrainDrive has some powerful features!"
- Be honest about issues
- Celebrate completion enthusiastically!

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
            return systemInfo.data;
          }
          return { error: systemInfo.error || 'Failed to detect system' };

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
          const ollamaResult = await this.hub.callBootstrapperTool('install_ollama', {}, 600000);
          return ollamaResult.data || ollamaResult;
        }

        case 'pull_ollama_model': {
          if (!this.hub.isBootstrapperConnected()) {
            return { error: 'Bootstrapper not connected' };
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
          if (input.repo_path) {
            payload.repo_path = input.repo_path;
          }
          const frontendDepsResult = await this.hub.callBootstrapperTool('install_frontend_deps', payload, 300000);
          return frontendDepsResult.data || frontendDepsResult;
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
          const startResult = await this.hub.callBootstrapperTool('start_braindrive', {
            frontend_port: input.frontend_port || 5173,
            backend_port: input.backend_port || 8005,
          }, 60000);
          if (startResult.success) {
            this.session.setBraindriveStatus('running');
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
