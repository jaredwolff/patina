(function () {
  "use strict";

  const messagesEl = document.getElementById("messages");
  const inputEl = document.getElementById("input");
  const formEl = document.getElementById("input-form");
  const sendBtn = document.getElementById("send-btn");
  const statusEl = document.getElementById("status");

  let ws = null;
  let chatId = localStorage.getItem("patina-session");
  let reconnectDelay = 1000;
  let thinkingEl = null;

  function setStatus(state, text) {
    statusEl.textContent = text || state;
    statusEl.className = "status " + state;
    sendBtn.disabled = state !== "connected";
  }

  function scrollToBottom() {
    messagesEl.scrollTop = messagesEl.scrollHeight;
  }

  function escapeHtml(str) {
    const div = document.createElement("div");
    div.textContent = str;
    return div.innerHTML;
  }

  // Minimal markdown to HTML renderer
  function renderMarkdown(text) {
    // Protect code blocks
    const codeBlocks = [];
    text = text.replace(/```(\w*)\n?([\s\S]*?)```/g, function (_, lang, code) {
      const idx = codeBlocks.length;
      codeBlocks.push(
        '<pre><code class="lang-' +
          escapeHtml(lang) +
          '">' +
          escapeHtml(code.replace(/\n$/, "")) +
          "</code></pre>"
      );
      return "\x00CB" + idx + "\x00";
    });

    // Protect inline code
    const inlineCode = [];
    text = text.replace(/`([^`\n]+)`/g, function (_, code) {
      const idx = inlineCode.length;
      inlineCode.push("<code>" + escapeHtml(code) + "</code>");
      return "\x00IC" + idx + "\x00";
    });

    // Escape HTML in remaining text
    text = escapeHtml(text);

    // Restore protected blocks (they were already escaped internally)
    text = text.replace(/\x00CB(\d+)\x00/g, function (_, idx) {
      return codeBlocks[parseInt(idx)];
    });
    text = text.replace(/\x00IC(\d+)\x00/g, function (_, idx) {
      return inlineCode[parseInt(idx)];
    });

    // Bold
    text = text.replace(/\*\*(.+?)\*\*/g, "<strong>$1</strong>");
    text = text.replace(/__(.+?)__/g, "<strong>$1</strong>");

    // Italic
    text = text.replace(/\*(.+?)\*/g, "<em>$1</em>");
    text = text.replace(/_(.+?)_/g, "<em>$1</em>");

    // Strikethrough
    text = text.replace(/~~(.+?)~~/g, "<s>$1</s>");

    // Links
    text = text.replace(
      /\[([^\]]+)\]\(([^)]+)\)/g,
      '<a href="$2" target="_blank" rel="noopener">$1</a>'
    );

    // Blockquotes
    text = text.replace(/^&gt; (.+)$/gm, "<blockquote>$1</blockquote>");

    // Unordered lists
    text = text.replace(/^[-*] (.+)$/gm, "<li>$1</li>");
    text = text.replace(/((?:<li>.*<\/li>\n?)+)/g, "<ul>$1</ul>");

    // Headers
    text = text.replace(/^#### (.+)$/gm, "<h4>$1</h4>");
    text = text.replace(/^### (.+)$/gm, "<h3>$1</h3>");
    text = text.replace(/^## (.+)$/gm, "<h2>$1</h2>");
    text = text.replace(/^# (.+)$/gm, "<h1>$1</h1>");

    // Paragraphs: split on double newlines
    const parts = text.split(/\n\n+/);
    text = parts
      .map(function (part) {
        part = part.trim();
        if (!part) return "";
        // Don't wrap block elements in <p>
        if (/^<(pre|ul|ol|h[1-4]|blockquote)/.test(part)) return part;
        return "<p>" + part.replace(/\n/g, "<br>") + "</p>";
      })
      .join("");

    return text;
  }

  function addMessage(role, content) {
    removeThinking();
    const div = document.createElement("div");
    div.className = "message " + role;
    if (role === "user" || role === "system") {
      div.textContent = content;
    } else {
      div.innerHTML = renderMarkdown(content);
    }
    messagesEl.appendChild(div);
    scrollToBottom();
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

  function connect() {
    setStatus("reconnecting", "connecting...");

    const proto = location.protocol === "https:" ? "wss:" : "ws:";
    let url = proto + "//" + location.host + "/ws";

    const params = [];
    const password = localStorage.getItem("patina-password");
    if (password) params.push("password=" + encodeURIComponent(password));
    if (chatId) params.push("session=" + encodeURIComponent(chatId));
    if (params.length) url += "?" + params.join("&");

    ws = new WebSocket(url);

    ws.onopen = function () {
      setStatus("connected", "connected");
      reconnectDelay = 1000;
    };

    ws.onmessage = function (evt) {
      let data;
      try {
        data = JSON.parse(evt.data);
      } catch (e) {
        return;
      }

      switch (data.type) {
        case "connected":
          chatId = data.chatId;
          localStorage.setItem("patina-session", chatId);
          break;
        case "message":
          addMessage("assistant", data.content);
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
        // Auth failure - don't auto-reconnect
        setStatus("disconnected", "auth failed");
        promptPassword();
        return;
      }

      setStatus("disconnected", "disconnected");
      // Auto-reconnect with exponential backoff
      setTimeout(connect, reconnectDelay);
      reconnectDelay = Math.min(reconnectDelay * 2, 30000);
    };

    ws.onerror = function () {
      // onclose will handle reconnection
    };
  }

  function promptPassword() {
    const pw = prompt("Enter password:");
    if (pw !== null) {
      localStorage.setItem("patina-password", pw);
      connect();
    }
  }

  function sendMessage(text) {
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    if (!text.trim()) return;

    addMessage("user", text);
    showThinking();

    ws.send(JSON.stringify({ type: "message", content: text }));
  }

  // Form submission
  formEl.addEventListener("submit", function (e) {
    e.preventDefault();
    const text = inputEl.value.trim();
    if (!text) return;
    sendMessage(text);
    inputEl.value = "";
    inputEl.style.height = "auto";
  });

  // Auto-resize textarea
  inputEl.addEventListener("input", function () {
    this.style.height = "auto";
    this.style.height = Math.min(this.scrollHeight, 120) + "px";
  });

  // Enter to send (Shift+Enter for newline)
  inputEl.addEventListener("keydown", function (e) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      formEl.dispatchEvent(new Event("submit"));
    }
  });

  // Start connection
  connect();
})();
