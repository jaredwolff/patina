import { render } from "preact";
import { App } from "./app";
import "./styles/global.css";
import "./styles/markdown.css";
import {
  loadSessions,
  syncSessions,
  sessions,
  activeChatId,
} from "./state/sessions";
import { loadPersonas } from "./state/personas";
import { connectWs } from "./state/websocket";

// Initialize state
loadSessions();
syncSessions();

if (sessions.value.length > 0) {
  activeChatId.value = sessions.value[0].id;
}

loadPersonas();
connectWs();

render(<App />, document.getElementById("app")!);
