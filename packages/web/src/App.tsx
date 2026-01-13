import { useEffect, useState, useRef } from 'react';
import './App.css';

interface Message {
  id: string;
  role: 'user' | 'assistant' | 'system';
  content: string;
  timestamp: Date;
  isStreaming?: boolean;
}

type ConnectionStatus = 'disconnected' | 'connecting' | 'connected';

function App() {
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState('');
  const [connectionStatus, setConnectionStatus] = useState<ConnectionStatus>('disconnected');
  const [bootstrapperConnected, setBootstrapperConnected] = useState(false);
  const [isTyping, setIsTyping] = useState(false);
  const wsRef = useRef<WebSocket | null>(null);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const streamingMessageIdRef = useRef<string | null>(null);

  const scrollToBottom = () => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  };

  useEffect(() => {
    scrollToBottom();
  }, [messages]);

  useEffect(() => {
    connectToBackend();
    return () => {
      if (wsRef.current) {
        wsRef.current.close();
      }
    };
  }, []);

  const connectToBackend = () => {
    setConnectionStatus('connecting');

    const ws = new WebSocket('ws://localhost:3000');
    wsRef.current = ws;

    ws.onopen = () => {
      setConnectionStatus('connected');
      ws.send(JSON.stringify({ type: 'browser_connect' }));

      // Add welcome message - user can chat immediately
      addMessage('system', 'Connected to BrainDrive Installation Server. You can start chatting now!');
    };

    ws.onmessage = (event) => {
      try {
        const data = JSON.parse(event.data);
        handleServerMessage(data);
      } catch (err) {
        console.error('Failed to parse message:', err);
      }
    };

    ws.onclose = () => {
      setConnectionStatus('disconnected');
      setBootstrapperConnected(false);
      addMessage('system', 'Disconnected from server. Reconnecting...');

      // Attempt to reconnect after 3 seconds
      setTimeout(connectToBackend, 3000);
    };

    ws.onerror = (err) => {
      console.error('WebSocket error:', err);
    };
  };

  const handleServerMessage = (data: any) => {
    switch (data.type) {
      case 'status_update':
        setBootstrapperConnected(data.bootstrapper_connected);
        if (data.bootstrapper_connected) {
          addMessage('system', 'Bootstrapper connected! You can now start the installation.');
        }
        break;

      case 'ai_message':
        // Legacy: full message at once (fallback)
        addMessage('assistant', data.content);
        break;

      case 'ai_message_start':
        // Start of a new streaming message
        const newId = `${Date.now()}-${Math.random().toString(36).substr(2, 9)}`;
        streamingMessageIdRef.current = newId;
        setMessages(prev => [...prev, {
          id: newId,
          role: 'assistant',
          content: '',
          timestamp: new Date(),
          isStreaming: true,
        }]);
        setIsTyping(false); // Hide typing indicator when text starts
        break;

      case 'ai_message_delta':
        // Streaming text chunk
        if (streamingMessageIdRef.current) {
          setMessages(prev => prev.map(msg =>
            msg.id === streamingMessageIdRef.current
              ? { ...msg, content: msg.content + data.content }
              : msg
          ));
        }
        break;

      case 'ai_message_end':
        // End of streaming message
        if (streamingMessageIdRef.current) {
          setMessages(prev => prev.map(msg =>
            msg.id === streamingMessageIdRef.current
              ? { ...msg, isStreaming: false }
              : msg
          ));
          streamingMessageIdRef.current = null;
        }
        break;

      case 'ai_typing':
        setIsTyping(data.typing);
        break;

      case 'tool_executing':
        addMessage('system', `Running: ${data.tool}`);
        break;

      case 'command_output':
        // Could show in a separate panel or append to messages
        console.log('Command output:', data.output);
        break;

      case 'error':
        addMessage('system', `Error: ${data.message}`);
        break;
    }
  };

  const addMessage = (role: 'user' | 'assistant' | 'system', content: string) => {
    setMessages(prev => [...prev, {
      id: `${Date.now()}-${Math.random().toString(36).substr(2, 9)}`,
      role,
      content,
      timestamp: new Date(),
    }]);
  };

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
