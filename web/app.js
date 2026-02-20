(function () {
  "use strict";

  var messagesEl = document.getElementById("messages");
  var inputEl = document.getElementById("input");
  var formEl = document.getElementById("input-form");
  var sendBtn = document.getElementById("send-btn");
  var statusEl = document.getElementById("status");
  var sessionListEl = document.getElementById("session-list");
  var newChatBtn = document.getElementById("new-chat-btn");
  var sidebarToggle = document.getElementById("sidebar-toggle");
  var appEl = document.getElementById("app");

  var ws = null;
  var reconnectDelay = 1000;
  var thinkingEl = null;

  // --- Session State ---

  var sessions = [];
  var activeChatId = null;
  var unreadChats = {};

  function loadSessions() {
    // Migrate from old single-session format
    var old = localStorage.getItem("patina-session");
    var stored = localStorage.getItem("patina-sessions");
    if (stored) {
      try {
        sessions = JSON.parse(stored);
      } catch (e) {
        sessions = [];
      }
    } else if (old) {
      sessions = [
        { id: old, title: "Chat", updatedAt: new Date().toISOString() },
      ];
      localStorage.removeItem("patina-session");
    }
    saveSessions();
  }

  function saveSessions() {
    localStorage.setItem("patina-sessions", JSON.stringify(sessions));
  }

  function findSession(id) {
    for (var i = 0; i < sessions.length; i++) {
      if (sessions[i].id === id) return sessions[i];
    }
    return null;
  }

  function generateUUID() {
    return "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx".replace(
      /[xy]/g,
      function (c) {
        var r = (Math.random() * 16) | 0;
        return (c === "x" ? r : (r & 0x3) | 0x8).toString(16);
      },
    );
  }

  // --- UI ---

  function setStatus(state, text) {
    statusEl.textContent = text || state;
    statusEl.className = "status " + state;
    sendBtn.disabled = state !== "connected";
  }

  function scrollToBottom() {
    messagesEl.scrollTop = messagesEl.scrollHeight;
  }

  function escapeHtml(str) {
    var div = document.createElement("div");
    div.textContent = str;
    return div.innerHTML;
  }

  function renderMarkdown(text) {
    var codeBlocks = [];
    text = text.replace(/```(\w*)\n?([\s\S]*?)```/g, function (_, lang, code) {
      var idx = codeBlocks.length;
      codeBlocks.push(
        '<pre><code class="lang-' +
          escapeHtml(lang) +
          '">' +
          escapeHtml(code.replace(/\n$/, "")) +
          "</code></pre>",
      );
      return "\x00CB" + idx + "\x00";
    });

    var inlineCode = [];
    text = text.replace(/`([^`\n]+)`/g, function (_, code) {
      var idx = inlineCode.length;
      inlineCode.push("<code>" + escapeHtml(code) + "</code>");
      return "\x00IC" + idx + "\x00";
    });

    text = escapeHtml(text);

    text = text.replace(/\x00CB(\d+)\x00/g, function (_, idx) {
      return codeBlocks[parseInt(idx)];
    });
    text = text.replace(/\x00IC(\d+)\x00/g, function (_, idx) {
      return inlineCode[parseInt(idx)];
    });

    text = text.replace(/\*\*(.+?)\*\*/g, "<strong>$1</strong>");
    text = text.replace(/__(.+?)__/g, "<strong>$1</strong>");
    text = text.replace(/\*(.+?)\*/g, "<em>$1</em>");
    text = text.replace(/_(.+?)_/g, "<em>$1</em>");
    text = text.replace(/~~(.+?)~~/g, "<s>$1</s>");
    text = text.replace(
      /\[([^\]]+)\]\(([^)]+)\)/g,
      '<a href="$2" target="_blank" rel="noopener">$1</a>',
    );
    text = text.replace(/^&gt; (.+)$/gm, "<blockquote>$1</blockquote>");
    text = text.replace(/^[-*] (.+)$/gm, "<li>$1</li>");
    text = text.replace(/((?:<li>.*<\/li>\n?)+)/g, "<ul>$1</ul>");
    text = text.replace(/^#### (.+)$/gm, "<h4>$1</h4>");
    text = text.replace(/^### (.+)$/gm, "<h3>$1</h3>");
    text = text.replace(/^## (.+)$/gm, "<h2>$1</h2>");
    text = text.replace(/^# (.+)$/gm, "<h1>$1</h1>");

    var parts = text.split(/\n\n+/);
    text = parts
      .map(function (part) {
        part = part.trim();
        if (!part) return "";
        if (/^<(pre|ul|ol|h[1-4]|blockquote)/.test(part)) return part;
        return "<p>" + part.replace(/\n/g, "<br>") + "</p>";
      })
      .join("");

    return text;
  }

  function addMessage(role, content) {
    removeThinking();
    var div = document.createElement("div");
    div.className = "message " + role;
    if (role === "user" || role === "system") {
      div.textContent = content;
    } else {
      div.innerHTML = renderMarkdown(content);
    }
    messagesEl.appendChild(div);
    scrollToBottom();
  }

  function clearMessages() {
    messagesEl.innerHTML = "";
  }

  function showThinking() {
    if (thinkingEl) return;
    thinkingEl = document.createElement("div");
    thinkingEl.className = "thinking";
    thinkingEl.innerHTML = "<span></span><span></span><span></span>";
    messagesEl.appendChild(thinkingEl);
    scrollToBottom();
  }

  function removeThinking() {
    if (thinkingEl) {
      thinkingEl.remove();
      thinkingEl = null;
    }
  }

  // --- Sidebar ---

  function renderSidebar() {
    sessionListEl.innerHTML = "";
    for (var i = 0; i < sessions.length; i++) {
      var s = sessions[i];
      var item = document.createElement("div");
      item.className =
        "session-item" + (s.id === activeChatId ? " active" : "");
      item.setAttribute("data-id", s.id);

      var titleDiv = document.createElement("div");
      titleDiv.className = "session-title";
      if (unreadChats[s.id]) {
        var dot = document.createElement("span");
        dot.className = "unread-dot";
        titleDiv.appendChild(dot);
      }
      var titleText = document.createElement("span");
      titleText.textContent = s.title || "New Chat";
      titleDiv.appendChild(titleText);
      item.appendChild(titleDiv);

      if (s.updatedAt) {
        var timeDiv = document.createElement("div");
        timeDiv.className = "session-time";
        timeDiv.textContent = formatTime(s.updatedAt);
        item.appendChild(timeDiv);
      }

      (function (id) {
        item.addEventListener("click", function () {
          if (id !== activeChatId) {
            switchChat(id);
          }
          closeSidebarMobile();
        });
      })(s.id);

      sessionListEl.appendChild(item);
    }
  }

  function formatTime(iso) {
    try {
      var d = new Date(iso);
      var now = new Date();
      if (d.toDateString() === now.toDateString()) {
        return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
      }
      return d.toLocaleDateString([], { month: "short", day: "numeric" });
    } catch (e) {
      return "";
    }
  }

  function closeSidebarMobile() {
    if (window.innerWidth <= 768) {
      appEl.classList.add("sidebar-hidden");
      removeOverlay();
    }
  }

  function removeOverlay() {
    var overlay = document.querySelector(".sidebar-overlay");
    if (overlay) overlay.remove();
  }

  // --- Chat Management ---

  function createNewChat() {
    var id = generateUUID();
    var session = {
      id: id,
      title: "New Chat",
      updatedAt: new Date().toISOString(),
    };
    sessions.unshift(session);
    saveSessions();
    switchChat(id);
    closeSidebarMobile();
  }

  function switchChat(id) {
    activeChatId = id;
    delete unreadChats[id];
    clearMessages();
    removeThinking();
    renderSidebar();

    // Request history for this chat from server
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "get_history", chatId: id }));
    }

    inputEl.focus();
  }

  function updateSessionTitle(id, firstMessage) {
    var s = findSession(id);
    if (s && s.title === "New Chat") {
      s.title =
        firstMessage.length > 50
          ? firstMessage.substring(0, 50) + "..."
          : firstMessage;
      saveSessions();
      renderSidebar();
    }
  }

  function updateSessionTime(id) {
    var s = findSession(id);
    if (s) {
      s.updatedAt = new Date().toISOString();
      saveSessions();
    }
  }

  // --- WebSocket ---

  function connectWs() {
    if (ws) return;

    setStatus("reconnecting", "connecting...");

    var proto = location.protocol === "https:" ? "wss:" : "ws:";
    var url = proto + "//" + location.host + "/ws";

    var password = localStorage.getItem("patina-password");
    if (password) url += "?password=" + encodeURIComponent(password);

    ws = new WebSocket(url);

    ws.onopen = function () {
      setStatus("connected", "connected");
      reconnectDelay = 1000;

      // Request history for the active chat now that we're connected
      if (activeChatId) {
        ws.send(JSON.stringify({ type: "get_history", chatId: activeChatId }));
      }
    };

    ws.onmessage = function (evt) {
      var data;
      try {
        data = JSON.parse(evt.data);
      } catch (e) {
        return;
      }

      switch (data.type) {
        case "connected":
          break;
        case "history":
          // Only render if this history is for the currently active chat
          if (
            data.chatId === activeChatId &&
            data.messages &&
            data.messages.length > 0
          ) {
            clearMessages();
            data.messages.forEach(function (msg) {
              addMessage(msg.role, msg.content);
            });
          }
          break;
        case "message":
          if (data.chatId === activeChatId) {
            addMessage("assistant", data.content);
          } else if (data.chatId) {
            unreadChats[data.chatId] = true;
            renderSidebar();
          }
          if (data.chatId) {
            updateSessionTime(data.chatId);
          }
          break;
        case "error":
          if (data.content && data.content.includes("Authentication")) {
            promptPassword();
          }
          addMessage("system", data.content || "Unknown error");
          break;
      }
    };

    ws.onclose = function (evt) {
      ws = null;
      removeThinking();

      if (evt.code === 4001) {
        setStatus("disconnected", "auth failed");
        promptPassword();
        return;
      }

      setStatus("disconnected", "disconnected");
      setTimeout(function () {
        connectWs();
      }, reconnectDelay);
      reconnectDelay = Math.min(reconnectDelay * 2, 30000);
    };

    ws.onerror = function () {};
  }

  function promptPassword() {
    var pw = prompt("Enter password:");
    if (pw !== null) {
      localStorage.setItem("patina-password", pw);
      if (ws) {
        ws.close();
        ws = null;
      }
      connectWs();
    }
  }

  function sendMessage(text) {
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    if (!text.trim()) return;

    addMessage("user", text);
    showThinking();
    updateSessionTitle(activeChatId, text);
    updateSessionTime(activeChatId);

    ws.send(
      JSON.stringify({ type: "message", content: text, chatId: activeChatId }),
    );
  }

  // --- Sync sessions from server ---

  function syncSessions() {
    fetch("/api/sessions")
      .then(function (res) {
        return res.json();
      })
      .then(function (serverSessions) {
        var localIds = {};
        sessions.forEach(function (s) {
          localIds[s.id] = true;
        });
        var added = false;
        serverSessions.forEach(function (s) {
          if (!localIds[s.id]) {
            sessions.push({
              id: s.id,
              title: s.title || "Chat",
              updatedAt: s.updatedAt || "",
            });
            added = true;
          }
        });
        if (added) {
          saveSessions();
          renderSidebar();
        }
      })
      .catch(function () {});
  }

  // --- Event Handlers ---

  formEl.addEventListener("submit", function (e) {
    e.preventDefault();
    var text = inputEl.value.trim();
    if (!text) return;
    sendMessage(text);
    inputEl.value = "";
    inputEl.style.height = "auto";
  });

  inputEl.addEventListener("input", function () {
    this.style.height = "auto";
    this.style.height = Math.min(this.scrollHeight, 120) + "px";
  });

  inputEl.addEventListener("keydown", function (e) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      formEl.dispatchEvent(new Event("submit"));
    }
  });

  newChatBtn.addEventListener("click", function () {
    createNewChat();
  });

  sidebarToggle.addEventListener("click", function () {
    if (appEl.classList.contains("sidebar-hidden")) {
      appEl.classList.remove("sidebar-hidden");
      var overlay = document.createElement("div");
      overlay.className = "sidebar-overlay";
      overlay.addEventListener("click", function () {
        closeSidebarMobile();
      });
      document.body.appendChild(overlay);
    } else {
      appEl.classList.add("sidebar-hidden");
      removeOverlay();
    }
  });

  // --- Init ---

  if (window.innerWidth <= 768) {
    appEl.classList.add("sidebar-hidden");
  }

  loadSessions();
  syncSessions();

  if (sessions.length === 0) {
    createNewChat();
  } else {
    activeChatId = sessions[0].id;
    renderSidebar();
  }

  // Single persistent connection — chat routing is per-message
  connectWs();
})();
