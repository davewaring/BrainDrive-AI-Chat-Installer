import express from 'express';
import { WebSocketServer } from 'ws';
import { createServer } from 'http';
import { config } from 'dotenv';
import { WebSocketHub } from './websocket-hub.js';
import { ClaudeClient } from './claude.js';
import { Session } from './session.js';

config();

const app = express();
const server = createServer(app);
const wss = new WebSocketServer({ server });

const PORT = process.env.PORT || 3000;

// Initialize components
const session = new Session();
const hub = new WebSocketHub(session);
const claude = new ClaudeClient(session, hub);

// CORS middleware for web frontend
app.use((req, res, next) => {
  res.header('Access-Control-Allow-Origin', '*');
  res.header('Access-Control-Allow-Headers', 'Content-Type');
  res.header('Access-Control-Allow-Methods', 'GET, POST, OPTIONS');
  if (req.method === 'OPTIONS') {
    return res.sendStatus(200);
  }
  next();
});

app.use(express.json());

// Health check endpoint
app.get('/health', (req, res) => {
  res.json({
    status: 'ok',
    session: session.getStatus(),
    bootstrapperConnected: hub.isBootstrapperConnected(),
  });
});

// WebSocket connection handler
wss.on('connection', (ws) => {
  console.log('New WebSocket connection');

  ws.on('message', async (data) => {
    try {
      const message = JSON.parse(data.toString());

      // Identify client type on first message
      if (message.type === 'browser_connect') {
        hub.setBrowserSocket(ws);
        ws.send(JSON.stringify({
          type: 'status_update',
          bootstrapper_connected: hub.isBootstrapperConnected(),
        }));
        console.log('Browser connected');
        return;
      }

      if (message.type === 'bootstrapper_connect') {
        hub.setBootstrapperSocket(ws);
        // Notify browser that bootstrapper is connected
        hub.sendToBrowser({
          type: 'status_update',
          bootstrapper_connected: true,
        });
        console.log('Bootstrapper connected');
        return;
      }

      // Handle user messages from browser
      if (message.type === 'user_message') {
        console.log('User message:', message.content);
        await claude.processMessage(message.content);
        return;
      }

      // Handle progress updates from bootstrapper - forward to browser
      if (message.type === 'progress') {
        hub.sendToBrowser({
          type: 'progress',
          id: message.id,
          operation: message.operation,
          percent: message.percent,
          message: message.message,
          bytes_downloaded: message.bytes_downloaded,
          bytes_total: message.bytes_total,
        });
        return;
      }

      // Handle responses from bootstrapper
      if (message.id && hub.hasPendingCall(message.id)) {
        hub.resolvePendingCall(message.id, message);
        return;
      }

    } catch (err) {
      console.error('Error processing message:', err);
      ws.send(JSON.stringify({
        type: 'error',
        message: err.message,
      }));
    }
  });

  ws.on('close', () => {
    hub.handleDisconnect(ws);
  });

  ws.on('error', (err) => {
    console.error('WebSocket error:', err);
  });
});

server.listen(PORT, () => {
  console.log(`Backend server running on port ${PORT}`);
  console.log(`WebSocket server ready at ws://localhost:${PORT}`);
});
