(function () {
  "use strict";

  // Configure marked: open links in new tabs
  var renderer = new marked.Renderer();
  var defaultLinkRenderer = renderer.link.bind(renderer);
  renderer.link = function (token) {
    var html = defaultLinkRenderer(token);
    return html.replace("<a ", '<a target="_blank" rel="noopener" ');
  };
  marked.setOptions({ renderer: renderer, gfm: true, breaks: true });

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
  var pageMenuBtn = document.getElementById("page-menu-btn");
  var pageMenu = document.getElementById("page-menu");
  var sidebarTitle = document.getElementById("sidebar-title");
  var chatAreaEl = document.getElementById("chat-area");
  var usageViewEl = document.getElementById("usage-view");
  var headerH1 = document.querySelector("#chat-area header h1");
  var chatIdEl = document.getElementById("chat-id");

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

  function getPersonaForSession(id) {
    var personaKey = getSessionPersona(id);
    if (!personaKey || !cachedPersonas) return null;
    for (var i = 0; i < cachedPersonas.length; i++) {
      if (cachedPersonas[i].key === personaKey) return cachedPersonas[i];
    }
    return null;
  }

  var PRESET_COLORS = [
    "#e74c3c",
    "#e67e22",
    "#f1c40f",
    "#2ecc71",
    "#1abc9c",
    "#3498db",
    "#9b59b6",
    "#e91e63",
    "#795548",
    "#607d8b",
  ];

  // --- UI ---

  function setStatus(state, text) {
    statusEl.textContent = text || state;
    statusEl.className = "status " + state;
    sendBtn.disabled = state !== "connected";
  }

  function scrollToBottom() {
    messagesEl.scrollTop = messagesEl.scrollHeight;
  }

  function renderMarkdown(text) {
    return marked.parse(text);
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

      // Avatar
      var persona = getPersonaForSession(s.id);
      var avatar = document.createElement("div");
      avatar.className = "session-avatar";
      avatar.style.background =
        persona && persona.color ? persona.color : "#888";
      avatar.textContent =
        persona && persona.name ? persona.name.charAt(0) : "P";
      item.appendChild(avatar);

      // Content wrapper
      var contentDiv = document.createElement("div");
      contentDiv.className = "session-item-content";

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
      contentDiv.appendChild(titleDiv);

      if (s.updatedAt) {
        var timeDiv = document.createElement("div");
        timeDiv.className = "session-time";
        timeDiv.textContent = formatTime(s.updatedAt);
        contentDiv.appendChild(timeDiv);
      }

      item.appendChild(contentDiv);

      // Delete button
      var delBtn = document.createElement("button");
      delBtn.className = "session-delete";
      delBtn.textContent = "\u00d7";
      delBtn.title = "Delete chat";
      (function (id) {
        delBtn.addEventListener("click", function (e) {
          e.stopPropagation();
          if (confirm("Delete this chat?")) {
            deleteChat(id);
          }
        });
      })(s.id);
      item.appendChild(delBtn);

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

  // --- Page Navigation ---

  var currentPage = "chats";

  function navigateTo(page) {
    currentPage = page;
    closePageMenu();

    if (page === "chats") {
      chatAreaEl.classList.remove("hidden");
      usageViewEl.classList.add("hidden");
      sidebarTitle.textContent = "Chats";
      sessionListEl.style.display = "";
      newChatBtn.style.display = "";
    } else if (page === "usage") {
      chatAreaEl.classList.add("hidden");
      usageViewEl.classList.remove("hidden");
      sidebarTitle.textContent = "Usage";
      sessionListEl.style.display = "none";
      newChatBtn.style.display = "none";
      loadUsageFilters();
      refreshUsage();
    }

    // Update active state on dropdown items
    pageMenu.querySelectorAll("[data-page]").forEach(function (item) {
      item.classList.toggle("active", item.getAttribute("data-page") === page);
    });

    closeSidebarMobile();
  }

  function closePageMenu() {
    pageMenu.classList.add("hidden");
  }

  pageMenuBtn.addEventListener("click", function (e) {
    e.stopPropagation();
    pageMenu.classList.toggle("hidden");
  });

  document.addEventListener("click", function (e) {
    if (
      !pageMenu.classList.contains("hidden") &&
      !pageMenu.contains(e.target) &&
      e.target !== pageMenuBtn
    ) {
      closePageMenu();
    }
  });

  pageMenu.querySelectorAll("[data-page]").forEach(function (item) {
    item.addEventListener("click", function () {
      navigateTo(item.getAttribute("data-page"));
    });
  });

  // --- Chat Management ---

  function deleteChat(id) {
    fetch("/api/sessions/" + encodeURIComponent(id), {
      method: "DELETE",
    }).catch(function () {});
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "delete_session", chatId: id }));
    }
    sessions = sessions.filter(function (s) {
      return s.id !== id;
    });
    delete unreadChats[id];
    saveSessions();

    if (id === activeChatId) {
      if (sessions.length > 0) {
        switchChat(sessions[0].id);
      } else {
        activeChatId = null;
        clearMessages();
        renderSidebar();
        createNewChat();
      }
    } else {
      renderSidebar();
    }
  }

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
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(
        JSON.stringify({
          type: "create_session",
          chatId: id,
          content: personaKey || "",
        }),
      );
    }
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
    chatIdEl.textContent = id ? id.slice(0, 8) : "";

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

    var persona = getPersonaForSession(activeChatId);
    if (persona) {
      var badge = document.createElement("span");
      badge.className = "persona-badge";

      var miniAvatar = document.createElement("span");
      miniAvatar.className = "header-avatar";
      miniAvatar.style.background = persona.color || "#888";
      miniAvatar.textContent = persona.name ? persona.name.charAt(0) : "P";
      badge.appendChild(miniAvatar);

      var nameSpan = document.createElement("span");
      nameSpan.textContent = persona.name || persona.key;
      badge.appendChild(nameSpan);

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
        case "user_message":
          if (data.chatId === activeChatId) {
            addMessage("user", data.content);
          }
          if (data.chatId) {
            updateSessionTitle(data.chatId, data.content);
            updateSessionTime(data.chatId);
            if (data.chatId !== activeChatId) {
              unreadChats[data.chatId] = true;
              renderSidebar();
            }
          }
          break;
        case "thinking":
          if (data.chatId === activeChatId) {
            showThinking();
          }
          break;
        case "session_created":
          if (data.chatId && !findSession(data.chatId)) {
            var personaKey = data.content || null;
            sessions.unshift({
              id: data.chatId,
              title: "New Chat",
              updatedAt: data.timestamp || new Date().toISOString(),
              persona: personaKey,
            });
            saveSessions();
            renderSidebar();
          }
          break;
        case "session_deleted":
          if (data.chatId) {
            sessions = sessions.filter(function (s) {
              return s.id !== data.chatId;
            });
            delete unreadChats[data.chatId];
            saveSessions();
            if (data.chatId === activeChatId) {
              if (sessions.length > 0) {
                switchChat(sessions[0].id);
              } else {
                activeChatId = null;
                clearMessages();
                renderSidebar();
                createNewChat();
              }
            } else {
              renderSidebar();
            }
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
        var serverIds = {};
        serverSessions.forEach(function (s) {
          serverIds[s.id] = s;
        });
        var localIds = {};
        sessions.forEach(function (s) {
          localIds[s.id] = true;
        });
        var changed = false;

        // Add server sessions missing locally
        serverSessions.forEach(function (s) {
          if (!localIds[s.id]) {
            sessions.push({
              id: s.id,
              title: s.title || "Chat",
              updatedAt: s.updatedAt || "",
              persona: s.persona || null,
            });
            changed = true;
          } else if (s.persona) {
            // Backfill persona for existing local sessions missing it
            var local = findSession(s.id);
            if (local && !local.persona) {
              local.persona = s.persona;
              changed = true;
            }
          }
        });

        // Remove local sessions that no longer exist on the server
        var before = sessions.length;
        sessions = sessions.filter(function (s) {
          return serverIds[s.id];
        });
        if (sessions.length !== before) {
          changed = true;
          // If active chat was removed, switch away
          if (activeChatId && !serverIds[activeChatId]) {
            if (sessions.length > 0) {
              switchChat(sessions[0].id);
            } else {
              activeChatId = null;
              clearMessages();
              createNewChat();
            }
          }
        }

        if (changed) {
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
      if (sessions.length === 0) {
        finishCreateChat(null);
      }
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

    // Populate color swatches
    var colorInput = document.getElementById("pe-color");
    var swatchesEl = document.getElementById("pe-color-swatches");
    var selectedColor = persona && persona.color ? persona.color : "";
    colorInput.value = selectedColor;
    swatchesEl.innerHTML = "";
    PRESET_COLORS.forEach(function (c) {
      var swatch = document.createElement("div");
      swatch.className =
        "color-swatch" + (c === selectedColor ? " selected" : "");
      swatch.style.background = c;
      swatch.addEventListener("click", function () {
        colorInput.value = c;
        swatchesEl.querySelectorAll(".color-swatch").forEach(function (s) {
          s.classList.remove("selected");
        });
        swatch.classList.add("selected");
      });
      swatchesEl.appendChild(swatch);
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
        color: document.getElementById("pe-color").value,
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

  function openSidebarMobile() {
    appEl.classList.remove("sidebar-hidden");
    var overlay = document.createElement("div");
    overlay.className = "sidebar-overlay";
    overlay.addEventListener("click", function () {
      closeSidebarMobile();
    });
    document.body.appendChild(overlay);
  }

  sidebarToggle.addEventListener("click", function () {
    if (appEl.classList.contains("sidebar-hidden")) {
      openSidebarMobile();
    } else {
      appEl.classList.add("sidebar-hidden");
      removeOverlay();
    }
  });

  document
    .getElementById("usage-sidebar-toggle")
    .addEventListener("click", function () {
      if (appEl.classList.contains("sidebar-hidden")) {
        openSidebarMobile();
      } else {
        appEl.classList.add("sidebar-hidden");
        removeOverlay();
      }
    });

  // --- Init ---

  if (window.innerWidth <= 768) {
    appEl.classList.add("sidebar-hidden");
  }

  chatIdEl.addEventListener("click", function () {
    if (activeChatId) {
      var sessionKey = "web:" + activeChatId;
      var ta = document.createElement("textarea");
      ta.value = sessionKey;
      ta.style.position = "fixed";
      ta.style.opacity = "0";
      document.body.appendChild(ta);
      ta.select();
      document.execCommand("copy");
      document.body.removeChild(ta);
      var prev = chatIdEl.textContent;
      chatIdEl.textContent = "copied!";
      setTimeout(function () {
        chatIdEl.textContent = prev;
      }, 1000);
    }
  });

  loadSessions();
  syncSessions();

  if (sessions.length === 0) {
    createNewChat();
  } else {
    switchChat(sessions[0].id);
  }

  // Pre-fetch personas so header badge and picker are ready
  fetchPersonas().then(function () {
    updateHeaderPersona();
    renderSidebar();
  });

  // Single persistent connection — chat routing is per-message
  connectWs();

  // --- Usage Dashboard ---

  var usageRefresh = document.getElementById("usage-refresh");
  var usageSortState = {
    table: "usage-table",
    key: "total_tokens",
    asc: false,
  };

  usageRefresh.addEventListener("click", function () {
    refreshUsage();
  });

  function getUsageParams() {
    var params = {};
    var from = document.getElementById("usage-from").value;
    var to = document.getElementById("usage-to").value;
    var model = document.getElementById("usage-model").value;
    var provider = document.getElementById("usage-provider").value;
    var agent = document.getElementById("usage-agent").value;
    var groupBy = document.getElementById("usage-group").value;
    if (from) params.from = from + "T00:00:00Z";
    if (to) params.to = to + "T23:59:59Z";
    if (model) params.model = model;
    if (provider) params.provider = provider;
    if (agent) params.agent = agent;
    if (groupBy) params.groupBy = groupBy;
    return params;
  }

  function buildQuery(params) {
    var parts = [];
    for (var k in params) {
      if (params.hasOwnProperty(k)) {
        parts.push(encodeURIComponent(k) + "=" + encodeURIComponent(params[k]));
      }
    }
    return parts.length ? "?" + parts.join("&") : "";
  }

  function formatTokens(n) {
    if (n === undefined || n === null) return "0";
    if (n >= 1000000) return (n / 1000000).toFixed(1) + "M";
    if (n >= 1000) return (n / 1000).toFixed(1) + "K";
    return String(n);
  }

  function formatCost(n) {
    if (n === null || n === undefined) return "\u2014";
    if (n < 0.01) return "<$0.01";
    return "$" + n.toFixed(2);
  }

  function loadUsageFilters() {
    fetch("/api/usage/filters")
      .then(function (r) {
        return r.json();
      })
      .then(function (data) {
        populateSelect("usage-model", data.models || []);
        populateSelect("usage-provider", data.providers || []);
        populateSelect("usage-agent", data.agents || []);
      })
      .catch(function () {});
  }

  function populateSelect(id, values) {
    var sel = document.getElementById(id);
    var current = sel.value;
    // Keep "All" option
    sel.innerHTML = '<option value="">All</option>';
    values.forEach(function (v) {
      var opt = document.createElement("option");
      opt.value = v;
      opt.textContent = v;
      sel.appendChild(opt);
    });
    sel.value = current;
  }

  function refreshUsage() {
    var params = getUsageParams();
    var qs = buildQuery(params);

    // Fetch summary cards (totals for today/week/month/all)
    fetchSummaryCards();

    // Fetch grouped summary
    fetch("/api/usage/summary" + qs)
      .then(function (r) {
        return r.json();
      })
      .then(function (rows) {
        if (rows && !rows.error) {
          renderUsageTable("usage-table", rows, "group_key");
        }
      })
      .catch(function () {});

    // Fetch daily breakdown
    var dailyParams = Object.assign({}, params);
    delete dailyParams.groupBy;
    var dailyQs = buildQuery(dailyParams);
    fetch("/api/usage/daily" + dailyQs)
      .then(function (r) {
        return r.json();
      })
      .then(function (rows) {
        if (rows && !rows.error) {
          renderUsageTable("usage-daily-table", rows, "date");
        }
      })
      .catch(function () {});
  }

  function fetchSummaryCards() {
    var now = new Date();
    var todayStr = now.toISOString().slice(0, 10);

    // Today
    var dayStart = todayStr + "T00:00:00Z";
    var dayEnd = todayStr + "T23:59:59Z";
    fetchTotalTokens({ from: dayStart, to: dayEnd }, "usage-today");

    // This week (Monday start)
    var weekDay = now.getDay();
    var mondayOffset = weekDay === 0 ? 6 : weekDay - 1;
    var monday = new Date(now);
    monday.setDate(now.getDate() - mondayOffset);
    var weekStart = monday.toISOString().slice(0, 10) + "T00:00:00Z";
    fetchTotalTokens({ from: weekStart, to: dayEnd }, "usage-week");

    // This month
    var monthStart = todayStr.slice(0, 7) + "-01T00:00:00Z";
    fetchTotalTokens({ from: monthStart, to: dayEnd }, "usage-month");

    // All time
    fetchTotalTokens({}, "usage-alltime");
  }

  function fetchTotalTokens(params, elementId) {
    var qs = buildQuery(params);
    fetch("/api/usage/daily" + qs)
      .then(function (r) {
        return r.json();
      })
      .then(function (rows) {
        var total = 0;
        var totalCost = 0;
        var hasCost = false;
        if (Array.isArray(rows)) {
          rows.forEach(function (r) {
            total += r.total_tokens || 0;
            if (r.estimated_cost != null) {
              totalCost += r.estimated_cost;
              hasCost = true;
            }
          });
        }
        var text = formatTokens(total);
        if (hasCost && totalCost > 0) {
          text += " (" + formatCost(totalCost) + ")";
        }
        document.getElementById(elementId).textContent = text;
      })
      .catch(function () {
        document.getElementById(elementId).textContent = "-";
      });
  }

  function renderUsageTable(tableId, rows, firstCol) {
    var tbody = document.querySelector("#" + tableId + " tbody");
    tbody.innerHTML = "";
    if (!rows || rows.length === 0) {
      var tr = document.createElement("tr");
      var td = document.createElement("td");
      td.colSpan = 7;
      td.textContent = "No data";
      td.style.textAlign = "center";
      td.style.color = "var(--text-secondary)";
      tr.appendChild(td);
      tbody.appendChild(tr);
      return;
    }
    rows.forEach(function (row) {
      var tr = document.createElement("tr");
      var fields = [
        firstCol,
        "calls",
        "input_tokens",
        "output_tokens",
        "total_tokens",
        "cached_input_tokens",
        "estimated_cost",
      ];
      fields.forEach(function (f) {
        var td = document.createElement("td");
        if (f === firstCol) {
          td.textContent = row[f] || "-";
        } else if (f === "estimated_cost") {
          td.textContent = formatCost(row[f]);
        } else {
          td.textContent = formatTokens(row[f]);
        }
        tr.appendChild(td);
      });
      tbody.appendChild(tr);
    });
  }

  // Sortable table headers
  document
    .querySelectorAll(
      "#usage-table th[data-sort], #usage-daily-table th[data-sort]",
    )
    .forEach(function (th) {
      th.style.cursor = "pointer";
      th.addEventListener("click", function () {
        var tableId = th.closest("table").id;
        var key = th.getAttribute("data-sort");
        var tbody = th.closest("table").querySelector("tbody");
        var rows = Array.prototype.slice.call(tbody.querySelectorAll("tr"));

        // Toggle direction
        var asc =
          usageSortState.table === tableId && usageSortState.key === key
            ? !usageSortState.asc
            : false;
        usageSortState = { table: tableId, key: key, asc: asc };

        // Update header indicators
        th.closest("thead")
          .querySelectorAll("th")
          .forEach(function (h) {
            h.classList.remove("sort-asc", "sort-desc");
          });
        th.classList.add(asc ? "sort-asc" : "sort-desc");

        rows.sort(function (a, b) {
          var colIdx = Array.prototype.indexOf.call(th.parentNode.children, th);
          var aVal = a.children[colIdx] ? a.children[colIdx].textContent : "";
          var bVal = b.children[colIdx] ? b.children[colIdx].textContent : "";
          // Try numeric comparison
          var aNum = parseFloat(aVal.replace(/[KM,]/g, ""));
          var bNum = parseFloat(bVal.replace(/[KM,]/g, ""));
          if (!isNaN(aNum) && !isNaN(bNum)) {
            // Restore actual values for proper comparison
            aNum = parseTokenValue(aVal);
            bNum = parseTokenValue(bVal);
            return asc ? aNum - bNum : bNum - aNum;
          }
          return asc ? aVal.localeCompare(bVal) : bVal.localeCompare(aVal);
        });

        rows.forEach(function (row) {
          tbody.appendChild(row);
        });
      });
    });

  function parseTokenValue(str) {
    str = str.trim();
    if (str.endsWith("M")) return parseFloat(str) * 1000000;
    if (str.endsWith("K")) return parseFloat(str) * 1000;
    return parseFloat(str) || 0;
  }
})();
