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
  var personaMgrBtn = document.getElementById("persona-mgr-btn");
  var headerH1 = document.querySelector("header h1");

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

  // --- Persona State & API ---

  var cachedPersonas = null;
  var cachedModelTiers = null;

  function fetchPersonas() {
    return fetch("/api/personas")
      .then(function (res) {
        return res.json();
      })
      .then(function (list) {
        cachedPersonas = list;
        return list;
      })
      .catch(function () {
        return [];
      });
  }

  function fetchModelTiers() {
    return fetch("/api/model-tiers")
      .then(function (res) {
        return res.json();
      })
      .then(function (tiers) {
        cachedModelTiers = tiers;
        return tiers;
      })
      .catch(function () {
        return ["default"];
      });
  }

  function createPersona(data) {
    return fetch("/api/personas", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(data),
    }).then(function (res) {
      if (!res.ok)
        return res.json().then(function (e) {
          throw new Error(e.error);
        });
      cachedPersonas = null;
      return res.json();
    });
  }

  function updatePersona(key, data) {
    return fetch("/api/personas/" + encodeURIComponent(key), {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(data),
    }).then(function (res) {
      if (!res.ok)
        return res.json().then(function (e) {
          throw new Error(e.error);
        });
      cachedPersonas = null;
      return res.json();
    });
  }

  function deletePersona(key) {
    return fetch("/api/personas/" + encodeURIComponent(key), {
      method: "DELETE",
    }).then(function (res) {
      if (!res.ok)
        return res.json().then(function (e) {
          throw new Error(e.error);
        });
      cachedPersonas = null;
      return res.json();
    });
  }

  function getSessionPersona(id) {
    var s = findSession(id);
    return s ? s.persona || null : null;
  }

  function getPersonaName(personaKey) {
    if (!personaKey || !cachedPersonas) return null;
    for (var i = 0; i < cachedPersonas.length; i++) {
      if (cachedPersonas[i].key === personaKey) return cachedPersonas[i].name;
    }
    return personaKey;
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
    // If personas exist, show picker first
    fetchPersonas().then(function (personas) {
      if (personas.length > 0) {
        showPersonaPicker(personas);
      } else {
        finishCreateChat(null);
      }
    });
  }

  function finishCreateChat(personaKey) {
    var id = generateUUID();
    var session = {
      id: id,
      title: "New Chat",
      updatedAt: new Date().toISOString(),
      persona: personaKey || null,
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
    updateHeaderPersona();

    // Request history for this chat from server
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "get_history", chatId: id }));
    }

    inputEl.focus();
  }

  function updateHeaderPersona() {
    // Remove existing badge
    var existing = headerH1.querySelector(".persona-badge");
    if (existing) existing.remove();

    var personaKey = getSessionPersona(activeChatId);
    if (personaKey) {
      var badge = document.createElement("span");
      badge.className = "persona-badge";
      badge.textContent = getPersonaName(personaKey) || personaKey;
      headerH1.appendChild(badge);
    }
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

    var msg = { type: "message", content: text, chatId: activeChatId };

    // Include persona on first message so gateway stores it in session metadata
    var personaKey = getSessionPersona(activeChatId);
    if (personaKey) {
      msg.persona = personaKey;
    }

    ws.send(JSON.stringify(msg));
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

  // --- Persona Picker ---

  function showPersonaPicker(personas) {
    var pickerEl = document.getElementById("persona-picker");
    var listEl = document.getElementById("persona-picker-list");
    listEl.innerHTML = "";

    personas.forEach(function (p) {
      var card = document.createElement("div");
      card.className = "persona-card";

      var name = document.createElement("div");
      name.className = "persona-card-name";
      name.textContent = p.name;
      card.appendChild(name);

      if (p.description) {
        var desc = document.createElement("div");
        desc.className = "persona-card-desc";
        desc.textContent = p.description;
        card.appendChild(desc);
      }

      if (p.modelTier && p.modelTier !== "default") {
        var tier = document.createElement("div");
        tier.className = "persona-card-tier";
        tier.textContent = "Model: " + p.modelTier;
        card.appendChild(tier);
      }

      card.addEventListener("click", function () {
        pickerEl.classList.add("hidden");
        finishCreateChat(p.key);
      });

      listEl.appendChild(card);
    });

    pickerEl.classList.remove("hidden");
  }

  document
    .getElementById("persona-picker-none")
    .addEventListener("click", function () {
      document.getElementById("persona-picker").classList.add("hidden");
      finishCreateChat(null);
    });

  document
    .getElementById("persona-picker-cancel")
    .addEventListener("click", function () {
      document.getElementById("persona-picker").classList.add("hidden");
    });

  // --- Persona Manager ---

  function showPersonaManager() {
    fetchPersonas().then(function (personas) {
      var managerEl = document.getElementById("persona-manager");
      var listEl = document.getElementById("persona-mgr-list");
      listEl.innerHTML = "";

      personas.forEach(function (p) {
        var item = document.createElement("div");
        item.className = "persona-mgr-item";

        var info = document.createElement("div");
        info.className = "persona-mgr-info";

        var name = document.createElement("div");
        name.className = "persona-mgr-name";
        name.textContent = p.name;
        info.appendChild(name);

        var desc = document.createElement("div");
        desc.className = "persona-mgr-desc";
        desc.textContent = p.description || p.key;
        info.appendChild(desc);

        item.appendChild(info);

        var actions = document.createElement("div");
        actions.className = "persona-mgr-actions";

        var editBtn = document.createElement("button");
        editBtn.className = "btn-text";
        editBtn.textContent = "Edit";
        editBtn.addEventListener("click", function () {
          managerEl.classList.add("hidden");
          showPersonaEditor(p);
        });
        actions.appendChild(editBtn);

        var delBtn = document.createElement("button");
        delBtn.className = "btn-danger";
        delBtn.textContent = "Delete";
        delBtn.addEventListener("click", function () {
          if (confirm('Delete persona "' + p.name + '"?')) {
            deletePersona(p.key).then(function () {
              showPersonaManager();
            });
          }
        });
        actions.appendChild(delBtn);

        item.appendChild(actions);
        listEl.appendChild(item);
      });

      if (personas.length === 0) {
        var empty = document.createElement("div");
        empty.style.cssText =
          "text-align:center;color:var(--text-secondary);font-size:13px;padding:16px 0;";
        empty.textContent = "No personas yet. Create one to get started.";
        listEl.appendChild(empty);
      }

      managerEl.classList.remove("hidden");
    });
  }

  document
    .getElementById("persona-mgr-close")
    .addEventListener("click", function () {
      document.getElementById("persona-manager").classList.add("hidden");
    });

  document
    .getElementById("persona-mgr-add")
    .addEventListener("click", function () {
      document.getElementById("persona-manager").classList.add("hidden");
      showPersonaEditor(null);
    });

  // --- Persona Editor ---

  var editingPersonaKey = null;

  function showPersonaEditor(persona) {
    var editorEl = document.getElementById("persona-editor");
    var titleEl = document.getElementById("persona-editor-title");
    var keyInput = document.getElementById("pe-key");
    var nameInput = document.getElementById("pe-name");
    var descInput = document.getElementById("pe-description");
    var preambleInput = document.getElementById("pe-preamble");
    var tierSelect = document.getElementById("pe-model-tier");

    editingPersonaKey = persona ? persona.key : null;
    titleEl.textContent = persona ? "Edit Persona" : "New Persona";

    keyInput.value = persona ? persona.key : "";
    keyInput.disabled = !!persona; // Can't change key on edit
    nameInput.value = persona ? persona.name : "";
    descInput.value = persona ? persona.description || "" : "";
    preambleInput.value = persona ? persona.preamble || "" : "";

    // Populate model tier dropdown
    var populateTiers = cachedModelTiers
      ? Promise.resolve(cachedModelTiers)
      : fetchModelTiers();

    populateTiers.then(function (tiers) {
      tierSelect.innerHTML = "";
      tiers.forEach(function (t) {
        var opt = document.createElement("option");
        opt.value = t;
        opt.textContent = t;
        if (persona && persona.modelTier === t) opt.selected = true;
        tierSelect.appendChild(opt);
      });
      // Add "default" if not present
      if (tiers.indexOf("default") === -1) {
        var defOpt = document.createElement("option");
        defOpt.value = "default";
        defOpt.textContent = "default";
        tierSelect.insertBefore(defOpt, tierSelect.firstChild);
      }
      if (!persona) tierSelect.value = "default";
    });

    editorEl.classList.remove("hidden");
    keyInput.focus();
  }

  document.getElementById("pe-cancel").addEventListener("click", function () {
    document.getElementById("persona-editor").classList.add("hidden");
  });

  document.getElementById("pe-generate").addEventListener("click", function () {
    var btn = this;
    var name = document.getElementById("pe-name").value.trim();
    if (!name) {
      alert("Enter a persona name first.");
      return;
    }
    var description = document.getElementById("pe-description").value.trim();

    btn.disabled = true;
    btn.textContent = "Generating...";

    fetch("/api/personas/generate-prompt", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name: name, description: description }),
    })
      .then(function (res) {
        return res.json();
      })
      .then(function (data) {
        if (data.error) {
          alert("Generation failed: " + data.error);
        } else if (data.preamble) {
          document.getElementById("pe-preamble").value = data.preamble;
        }
      })
      .catch(function (err) {
        alert("Generation failed: " + err.message);
      })
      .finally(function () {
        btn.disabled = false;
        btn.textContent = "Generate";
      });
  });

  document
    .getElementById("persona-editor-form")
    .addEventListener("submit", function (e) {
      e.preventDefault();

      var key = document.getElementById("pe-key").value.trim();
      var data = {
        key: key,
        name: document.getElementById("pe-name").value.trim(),
        description: document.getElementById("pe-description").value.trim(),
        preamble: document.getElementById("pe-preamble").value,
        modelTier: document.getElementById("pe-model-tier").value,
      };

      var promise = editingPersonaKey
        ? updatePersona(editingPersonaKey, data)
        : createPersona(data);

      promise
        .then(function () {
          document.getElementById("persona-editor").classList.add("hidden");
          showPersonaManager();
          // Refresh cached personas so badge/picker stay current
          fetchPersonas().then(function () {
            updateHeaderPersona();
          });
        })
        .catch(function (err) {
          alert("Error: " + err.message);
        });
    });

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

  personaMgrBtn.addEventListener("click", function () {
    showPersonaManager();
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

  // Pre-fetch personas so header badge and picker are ready
  fetchPersonas().then(function () {
    updateHeaderPersona();
  });

  // Single persistent connection — chat routing is per-message
  connectWs();
})();
