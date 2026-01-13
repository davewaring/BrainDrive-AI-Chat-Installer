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
- Detect what software is installed on their system
- Run commands to install dependencies
- Start, stop, and restart BrainDrive
- Check if ports are available

## Installation Flow
1. First, greet the user warmly
2. Explain what BrainDrive is and what the installer does
3. Ask them to download the bootstrapper app (they'll see a download button)
4. Once the bootstrapper connects, detect their system to understand what's already installed
5. Guide them through installing any missing dependencies (conda, git, node)
6. Clone the BrainDrive repository
7. Set up the environment and install dependencies
8. Start BrainDrive and celebrate!

## Important Guidelines
- If the bootstrapper is not connected, you can still chat! Explain the process and guide them to download it.
- Always explain what you're about to do BEFORE doing it
- After each major step, confirm it succeeded before moving on
- If a command fails, explain what went wrong in simple terms
- Never run destructive commands without explicit user confirmation
- Keep the user informed during long-running operations

## Conversation Style
- Use short paragraphs and bullet points for clarity
- Add occasional encouraging messages during waits
- If the user seems confused, offer to explain in more detail
- Be honest about any limitations or issues

Remember: Your goal is to make the installation process feel easy and even enjoyable. The user should feel supported every step of the way.`;

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

        case 'run_command':
          if (!this.hub.isBootstrapperConnected()) {
            return { error: 'Bootstrapper not connected' };
          }
          const result = await this.hub.callBootstrapperTool('run_command', {
            command: input.command,
          }, 300000); // 5 minute timeout for commands
          if (result.success !== undefined) {
            return result.data || result;
          }
          return result;

        case 'check_port_available':
          if (!this.hub.isBootstrapperConnected()) {
            return { error: 'Bootstrapper not connected' };
          }
          const portResult = await this.hub.callBootstrapperTool('check_port', {
            port: input.port,
          }, 10000);
          return portResult.data || portResult;

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
