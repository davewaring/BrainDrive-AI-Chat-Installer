import { v4 as uuidv4 } from 'uuid';

export class Session {
  constructor() {
    this.id = uuidv4();
    this.browserConnected = false;
    this.bootstrapperConnected = false;
    this.conversationHistory = [];
    this.systemInfo = null;
    this.installState = 'not_started'; // not_started | in_progress | completed | failed
    this.braindriveStatus = 'unknown'; // unknown | stopped | running
    this.createdAt = new Date();
  }

  setBrowserConnected(connected) {
    this.browserConnected = connected;
  }

  setBootstrapperConnected(connected) {
    this.bootstrapperConnected = connected;
  }

  addMessage(role, content) {
    this.conversationHistory.push({
      role,
      content,
      timestamp: new Date(),
    });
  }

  getConversationHistory() {
    return this.conversationHistory.map(msg => ({
      role: msg.role,
      content: msg.content,
    }));
  }

  setSystemInfo(info) {
    this.systemInfo = info;
  }

  setInstallState(state) {
    this.installState = state;
  }

  setBraindriveStatus(status) {
    this.braindriveStatus = status;
  }

  getStatus() {
    return {
      id: this.id,
      browserConnected: this.browserConnected,
      bootstrapperConnected: this.bootstrapperConnected,
      installState: this.installState,
      braindriveStatus: this.braindriveStatus,
      messageCount: this.conversationHistory.length,
      createdAt: this.createdAt,
    };
  }

  reset() {
    this.conversationHistory = [];
    this.systemInfo = null;
    this.installState = 'not_started';
    this.braindriveStatus = 'unknown';
  }
}
