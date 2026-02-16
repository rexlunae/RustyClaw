// RustyClaw Web UI - Progressive Web App
// Mobile-friendly control dashboard

class RustyClawApp {
    constructor() {
        this.ws = null;
        this.connected = false;
        this.sessionId = null;
        this.messages = [];
        this.sessions = [];

        this.init();
    }

    init() {
        // DOM elements
        this.statusDot = document.getElementById('statusDot');
        this.statusText = document.getElementById('statusText');
        this.messagesContainer = document.getElementById('messages');
        this.messageInput = document.getElementById('messageInput');
        this.sendBtn = document.getElementById('sendBtn');
        this.gatewayUrl = document.getElementById('gatewayUrl');

        // Event listeners
        this.setupEventListeners();

        // Auto-resize textarea
        this.messageInput.addEventListener('input', () => {
            this.messageInput.style.height = 'auto';
            this.messageInput.style.height = this.messageInput.scrollHeight + 'px';
        });

        // Load saved gateway URL
        const savedUrl = localStorage.getItem('gatewayUrl');
        if (savedUrl) {
            this.gatewayUrl.value = savedUrl;
        }

        // Auto-connect if previously connected
        const autoConnect = localStorage.getItem('autoConnect');
        if (autoConnect === 'true') {
            this.connect();
        }
    }

    setupEventListeners() {
        // Tab switching
        document.querySelectorAll('.tab').forEach(tab => {
            tab.addEventListener('click', () => this.switchTab(tab.dataset.tab));
        });

        // Send message
        this.sendBtn.addEventListener('click', () => this.sendMessage());
        this.messageInput.addEventListener('keypress', (e) => {
            if (e.key === 'Enter' && !e.shiftKey) {
                e.preventDefault();
                this.sendMessage();
            }
        });
    }

    switchTab(tabName) {
        // Update tab styles
        document.querySelectorAll('.tab').forEach(t => t.classList.remove('active'));
        document.querySelector(`[data-tab="${tabName}"]`).classList.add('active');

        // Update content
        document.querySelectorAll('.tab-content').forEach(c => c.classList.remove('active'));
        document.getElementById(`${tabName}-tab`).classList.add('active');

        // Refresh content for specific tabs
        if (tabName === 'sessions') {
            this.refreshSessions();
        }
    }

    connect() {
        const url = this.gatewayUrl.value || 'ws://localhost:8080';

        // Save URL
        localStorage.setItem('gatewayUrl', url);
        localStorage.setItem('autoConnect', 'true');

        this.updateStatus('Connecting...', false);

        try {
            this.ws = new WebSocket(url);

            this.ws.onopen = () => {
                this.connected = true;
                this.updateStatus('Connected', true);
                this.sendBtn.disabled = false;
                this.clearEmptyState();
                console.log('WebSocket connected, waiting for hello...');
            };

            this.ws.onmessage = (event) => {
                this.handleMessage(event.data);
            };

            this.ws.onerror = (error) => {
                console.error('WebSocket error:', error);
                this.updateStatus('Error', false);
            };

            this.ws.onclose = () => {
                this.connected = false;
                this.updateStatus('Disconnected', false);
                this.sendBtn.disabled = true;

                // Auto-reconnect after 5 seconds
                setTimeout(() => {
                    if (localStorage.getItem('autoConnect') === 'true') {
                        this.connect();
                    }
                }, 5000);
            };

        } catch (error) {
            console.error('Connection error:', error);
            this.updateStatus('Failed to connect', false);
        }
    }

    disconnect() {
        localStorage.setItem('autoConnect', 'false');
        if (this.ws) {
            this.ws.close();
        }
    }

    send(data) {
        if (this.ws && this.connected) {
            this.ws.send(JSON.stringify(data));
        }
    }

    handleMessage(data) {
        try {
            const msg = JSON.parse(data);

            switch (msg.type) {
                case 'hello':
                    this.addSystemMessage(`Connected to ${msg.agent} (${msg.provider || 'no model'})`);
                    console.log('Gateway hello:', msg);
                    break;

                case 'response_chunk':
                    // Streaming text from assistant
                    this.appendToLastMessage(msg.chunk);
                    break;

                case 'response_done':
                    // Response complete
                    console.log('Response complete');
                    break;

                case 'tool_call':
                    this.addSystemMessage(`🔧 ${msg.name}(${JSON.stringify(msg.arguments).substring(0, 50)}...)`);
                    break;

                case 'tool_result':
                    const status = msg.is_error ? '❌' : '✅';
                    this.addSystemMessage(`${status} ${msg.name}: ${msg.result.substring(0, 100)}...`);
                    break;

                case 'error':
                    this.addSystemMessage('❌ Error: ' + msg.message);
                    break;

                case 'info':
                    this.addSystemMessage('ℹ️ ' + msg.message);
                    break;

                default:
                    console.log('Unknown message type:', msg);
            }
        } catch (error) {
            console.error('Failed to parse message:', error, data);
        }
    }

    sendMessage() {
        const content = this.messageInput.value.trim();
        if (!content || !this.connected) return;

        // Add user message to UI
        this.addMessage({
            role: 'user',
            content: content,
            timestamp: Date.now()
        });

        // Prepare assistant placeholder for streaming response
        this.addMessage({
            role: 'assistant',
            content: '',
            timestamp: Date.now()
        });

        // Send to gateway (RustyClaw protocol)
        this.send({
            type: 'chat',
            messages: this.messages.map(m => ({
                role: m.role,
                content: m.content
            }))
        });

        // Clear input
        this.messageInput.value = '';
        this.messageInput.style.height = 'auto';
    }

    addMessage(msg) {
        this.messages.push(msg);

        const messageEl = document.createElement('div');
        messageEl.className = `message ${msg.role || 'system'}`;
        messageEl.dataset.messageIndex = this.messages.length - 1;

        const header = document.createElement('div');
        header.className = 'message-header';

        const role = document.createElement('span');
        role.textContent = this.formatRole(msg.role);
        header.appendChild(role);

        const time = document.createElement('span');
        time.textContent = this.formatTime(msg.timestamp);
        header.appendChild(time);

        const content = document.createElement('div');
        content.className = 'message-content';
        content.textContent = msg.content;

        messageEl.appendChild(header);
        messageEl.appendChild(content);

        this.messagesContainer.appendChild(messageEl);

        // Scroll to bottom
        this.messagesContainer.scrollTop = this.messagesContainer.scrollHeight;
    }

    appendToLastMessage(chunk) {
        if (this.messages.length === 0) return;

        const lastMsg = this.messages[this.messages.length - 1];
        lastMsg.content += chunk;

        // Update DOM
        const messageEls = this.messagesContainer.querySelectorAll('.message');
        const lastEl = messageEls[messageEls.length - 1];
        if (lastEl) {
            const contentEl = lastEl.querySelector('.message-content');
            if (contentEl) {
                contentEl.textContent = lastMsg.content;
            }
        }

        // Scroll to bottom
        this.messagesContainer.scrollTop = this.messagesContainer.scrollHeight;
    }

    addSystemMessage(text) {
        this.addMessage({
            role: 'system',
            content: text,
            timestamp: Date.now()
        });
    }

    clearEmptyState() {
        const emptyState = this.messagesContainer.querySelector('.empty-state');
        if (emptyState) {
            emptyState.remove();
        }
    }

    refreshSessions() {
        const container = document.getElementById('sessions');

        if (this.sessions.length === 0) {
            container.innerHTML = `
                <div class="empty-state">
                    <div class="empty-state-icon">📋</div>
                    <p>No active sessions</p>
                </div>
            `;
            return;
        }

        container.innerHTML = '';
        this.sessions.forEach(session => {
            const card = document.createElement('div');
            card.className = 'session-card';

            const header = document.createElement('div');
            header.className = 'session-header';
            header.textContent = session.name || `Session ${session.id.substring(0, 8)}`;

            const meta = document.createElement('div');
            meta.className = 'session-meta';
            meta.textContent = `Messages: ${session.message_count || 0} • ${this.formatTime(session.created_at)}`;

            card.appendChild(header);
            card.appendChild(meta);
            container.appendChild(card);
        });
    }

    updateStatus(text, connected) {
        this.statusText.textContent = text;
        if (connected) {
            this.statusDot.classList.add('connected');
        } else {
            this.statusDot.classList.remove('connected');
        }
    }

    formatRole(role) {
        const roles = {
            'user': 'You',
            'assistant': 'RustyClaw',
            'system': 'System'
        };
        return roles[role] || role;
    }

    formatTime(timestamp) {
        if (!timestamp) return '';
        const date = new Date(timestamp);
        const now = new Date();

        // Today: show time only
        if (date.toDateString() === now.toDateString()) {
            return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
        }

        // This year: show date and time
        if (date.getFullYear() === now.getFullYear()) {
            return date.toLocaleDateString([], { month: 'short', day: 'numeric' }) +
                   ' ' + date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
        }

        // Other years: full date
        return date.toLocaleDateString();
    }
}

// Initialize app
const app = new RustyClawApp();

// Register service worker for PWA
if ('serviceWorker' in navigator) {
    window.addEventListener('load', () => {
        navigator.serviceWorker.register('sw.js')
            .then(reg => console.log('Service Worker registered'))
            .catch(err => console.log('Service Worker registration failed:', err));
    });
}
