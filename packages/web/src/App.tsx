import { useCallback, useEffect, useRef, useState } from 'react';
import './App.css';

interface Message {
  id: string;
  role: 'user' | 'assistant' | 'system';
  content: string;
  timestamp: Date;
  isStreaming?: boolean;
}

type ConnectionStatus = 'disconnected' | 'connecting' | 'connected';

type ServerMessage =
  | { type: 'status_update'; bootstrapper_connected: boolean }
  | { type: 'ai_message'; content: string }
  | { type: 'ai_message_start' }
  | { type: 'ai_message_delta'; content: string }
  | { type: 'ai_message_end' }
  | { type: 'ai_typing'; typing: boolean }
  | { type: 'tool_executing'; tool: string }
  | { type: 'command_output'; output: string }
  | { type: 'error'; message: string }
  | { type: string; [key: string]: unknown };

function App() {
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState('');
  const [connectionStatus, setConnectionStatus] = useState<ConnectionStatus>('disconnected');
  const [bootstrapperConnected, setBootstrapperConnected] = useState(false);
  const [isTyping, setIsTyping] = useState(false);
  const wsRef = useRef<WebSocket | null>(null);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const streamingMessageIdRef = useRef<string | null>(null);
  const messageIdRef = useRef(0);
  const reconnectRef = useRef<() => void>(() => {});

  const scrollToBottom = () => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  };

  useEffect(() => {
    scrollToBottom();
  }, [messages]);

  const createMessageId = useCallback(() => {
    messageIdRef.current += 1;
    return `msg-${messageIdRef.current}`;
  }, []);

  const addMessage = useCallback((role: 'user' | 'assistant' | 'system', content: string) => {
    const id = createMessageId();
    setMessages(prev => [
      ...prev,
      {
        id,
        role,
        content,
        timestamp: new Date(),
      },
    ]);
  }, [createMessageId]);

  const handleServerMessage = useCallback((data: ServerMessage) => {
    switch (data.type) {
      case 'status_update': {
        const connected = Boolean(data.bootstrapper_connected);
        setBootstrapperConnected(connected);
        if (connected) {
          addMessage('system', 'Bootstrapper connected! You can now start the installation.');
        }
        break;
      }

      case 'ai_message': {
        addMessage('assistant', String(data.content ?? ''));
        break;
      }

      case 'ai_message_start': {
        const newId = createMessageId();
        streamingMessageIdRef.current = newId;
        setMessages(prev => [
          ...prev,
          {
            id: newId,
            role: 'assistant',
            content: '',
            timestamp: new Date(),
            isStreaming: true,
          },
        ]);
        setIsTyping(false);
        break;
      }

      case 'ai_message_delta': {
        if (streamingMessageIdRef.current) {
          setMessages(prev => prev.map(msg =>
            msg.id === streamingMessageIdRef.current
              ? { ...msg, content: `${msg.content}${data.content ?? ''}` }
              : msg
          ));
        }
        break;
      }

      case 'ai_message_end': {
        if (streamingMessageIdRef.current) {
          const currentId = streamingMessageIdRef.current;
          setMessages(prev => prev.map(msg =>
            msg.id === currentId
              ? { ...msg, isStreaming: false }
              : msg
          ));
          streamingMessageIdRef.current = null;
        }
        break;
      }

      case 'ai_typing': {
        setIsTyping(Boolean(data.typing));
        break;
      }

      case 'tool_executing': {
        addMessage('system', `Running: ${String(data.tool ?? 'helper')}`);
        break;
      }

      case 'command_output': {
        console.log('Command output:', data.output);
        break;
      }

      case 'error': {
        addMessage('system', `Error: ${String(data.message ?? 'Unknown error')}`);
        break;
      }

      default:
        break;
    }
  }, [addMessage, createMessageId]);

  const connectToBackend = useCallback(() => {
    setConnectionStatus('connecting');

    const ws = new WebSocket('ws://localhost:3000');
    wsRef.current = ws;

    ws.onopen = () => {
      setConnectionStatus('connected');
      ws.send(JSON.stringify({ type: 'browser_connect' }));
      addMessage('system', 'Connected to BrainDrive Installation Server. You can start chatting now!');
    };

    ws.onmessage = (event) => {
      try {
        const parsed = JSON.parse(event.data) as ServerMessage;
        handleServerMessage(parsed);
      } catch (err) {
        console.error('Failed to parse message:', err);
      }
    };

    ws.onclose = () => {
      setConnectionStatus('disconnected');
      setBootstrapperConnected(false);
      addMessage('system', 'Disconnected from server. Reconnecting...');
      setTimeout(() => reconnectRef.current(), 3000);
    };

    ws.onerror = (err) => {
      console.error('WebSocket error:', err);
    };
  }, [addMessage, handleServerMessage]);

  useEffect(() => {
    reconnectRef.current = connectToBackend;
  }, [connectToBackend]);

  useEffect(() => {
    const timer = window.setTimeout(() => connectToBackend(), 0);
    return () => {
      window.clearTimeout(timer);
      if (wsRef.current) {
        wsRef.current.close();
      }
    };
  }, [connectToBackend]);

  const sendMessage = () => {
    if (!input.trim() || !wsRef.current || connectionStatus !== 'connected') {
      return;
    }

    const content = input.trim();
    addMessage('user', content);
    setInput('');

    wsRef.current.send(JSON.stringify({
      type: 'user_message',
      content,
    }));
  };

  const handleKeyPress = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      sendMessage();
    }
  };

  const getStatusColor = () => {
    if (connectionStatus !== 'connected') return 'status-disconnected';
    if (!bootstrapperConnected) return 'status-waiting';
    return 'status-ready';
  };

  const getStatusText = () => {
    if (connectionStatus === 'disconnected') return 'Disconnected';
    if (connectionStatus === 'connecting') return 'Connecting...';
    if (!bootstrapperConnected) return 'Waiting for Bootstrapper';
    return 'Ready';
  };

  return (
    <div className="app">
      <header className="header">
        <div className="header-content">
          <h1>BrainDrive</h1>
          <p className="tagline">AI-Powered Installation</p>
        </div>
        <div className={`status-badge ${getStatusColor()}`}>
          <span className="status-dot"></span>
          <span>{getStatusText()}</span>
        </div>
      </header>

      {!bootstrapperConnected && connectionStatus === 'connected' && (
        <div className="download-banner">
          <p>To begin installation, download and open the BrainDrive Bootstrapper:</p>
          <div className="download-buttons">
            <button className="download-btn">
              Download for macOS
            </button>
            <button className="download-btn" disabled>
              Windows (Coming Soon)
            </button>
          </div>
        </div>
      )}

      <main className="chat-container">
        <div className="messages">
          {messages.map((msg) => (
            <div key={msg.id} className={`message message-${msg.role}`}>
              <div className="message-content">
                {msg.content}
              </div>
              <div className="message-time">
                {msg.timestamp.toLocaleTimeString()}
              </div>
            </div>
          ))}
          {isTyping && (
            <div className="message message-assistant">
              <div className="message-content typing">
                <span className="typing-dot"></span>
                <span className="typing-dot"></span>
                <span className="typing-dot"></span>
              </div>
            </div>
          )}
          <div ref={messagesEndRef} />
        </div>
      </main>

      <footer className="input-container">
        <div className="input-wrapper">
          <textarea
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyPress={handleKeyPress}
            placeholder={connectionStatus === 'connected' ? "Type a message..." : "Connecting to server..."}
            disabled={connectionStatus !== 'connected'}
            rows={1}
          />
          <button
            onClick={sendMessage}
            disabled={!input.trim() || connectionStatus !== 'connected'}
            className="send-btn"
          >
            Send
          </button>
        </div>
      </footer>
    </div>
  );
}

export default App;
