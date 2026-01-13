export class WebSocketHub {
  constructor(session) {
    this.session = session;
    this.browserSocket = null;
    this.bootstrapperSocket = null;
    this.pendingCalls = new Map();
  }

  setBrowserSocket(ws) {
    this.browserSocket = ws;
    this.session.setBrowserConnected(true);
  }

  setBootstrapperSocket(ws) {
    this.bootstrapperSocket = ws;
    this.session.setBootstrapperConnected(true);
  }

  isBootstrapperConnected() {
    return this.bootstrapperSocket !== null &&
           this.bootstrapperSocket.readyState === 1; // WebSocket.OPEN
  }

  isBrowserConnected() {
    return this.browserSocket !== null &&
           this.browserSocket.readyState === 1;
  }

  handleDisconnect(ws) {
    if (ws === this.browserSocket) {
      this.browserSocket = null;
      this.session.setBrowserConnected(false);
      console.log('Browser disconnected');
    }
    if (ws === this.bootstrapperSocket) {
      this.bootstrapperSocket = null;
      this.session.setBootstrapperConnected(false);
      // Notify browser
      this.sendToBrowser({
        type: 'status_update',
        bootstrapper_connected: false,
      });
      console.log('Bootstrapper disconnected');
    }
  }

  sendToBrowser(message) {
    if (this.isBrowserConnected()) {
      this.browserSocket.send(JSON.stringify(message));
    }
  }

  sendToBootstrapper(message) {
    if (this.isBootstrapperConnected()) {
      this.bootstrapperSocket.send(JSON.stringify(message));
    }
  }

  hasPendingCall(id) {
    return this.pendingCalls.has(id);
  }

  resolvePendingCall(id, result) {
    const pending = this.pendingCalls.get(id);
    if (pending) {
      clearTimeout(pending.timeout);
      pending.resolve(result);
      this.pendingCalls.delete(id);
    }
  }

  /**
   * Call a tool on the bootstrapper and wait for response
   * @param {string} type - Tool type (e.g., 'detect_system', 'install_conda_env')
   * @param {object} params - Tool parameters
   * @param {number} timeoutMs - Timeout in milliseconds
   * @returns {Promise<object>} - Tool result
   */
  async callBootstrapperTool(type, params, timeoutMs = 30000) {
    if (!this.isBootstrapperConnected()) {
      throw new Error('Bootstrapper not connected');
    }

    const id = `tool_${Date.now()}_${Math.random().toString(36).substr(2, 9)}`;

    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        this.pendingCalls.delete(id);
        reject(new Error(`Tool call ${type} timed out after ${timeoutMs}ms`));
      }, timeoutMs);

      this.pendingCalls.set(id, { resolve, reject, timeout });

      this.sendToBootstrapper({
        type,
        id,
        ...params,
      });
    });
  }

  /**
   * Stream command output from bootstrapper to browser
   * @param {string} output - Output line
   * @param {string} command - Command being executed
   */
  streamCommandOutput(output, command) {
    this.sendToBrowser({
      type: 'command_output',
      output,
      command,
    });
  }
}
