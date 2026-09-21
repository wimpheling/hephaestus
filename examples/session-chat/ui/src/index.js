import { SessionGitClient } from "./git-client.js";
import { makeHumanMessage } from "./protocol.js";
import { loadUiContext } from "./ui-context.js";
import "./style.css";

const root = document.querySelector("[data-heph-session-chat]");
const status = root?.querySelector("[data-status]");
const transcript = root?.querySelector("[data-transcript]");
const form = root?.querySelector("form");
const input = root?.querySelector("textarea");
const reconnect = root?.querySelector("[data-reconnect]");
let client;

function setStatus(message, error = false) {
  if (status) {
    status.textContent = message;
    status.dataset.state = error ? "error" : "ready";
  }
}

function render(records) {
  transcript.replaceChildren();
  for (const record of records) {
    const item = document.createElement("article");
    item.className = `message message-${record.kind === "user_message" ? "human" : "agent"}`;
    const heading = document.createElement("h2");
    heading.textContent = record.actor.role === "human" ? "You" : "Assistant";
    const body = document.createElement("p");
    body.textContent = record.content.kind === "text"
      ? record.content.text
      : `[${record.content.media_type}, ${record.content.byte_length} bytes, sha256 ${record.content.sha256}]`;
    const time = document.createElement("time");
    time.dateTime = record.created_at;
    time.textContent = new Date(record.created_at).toLocaleString();
    item.append(heading, body, time);
    transcript.append(item);
  }
}

async function refresh() {
  const session = await client.reconnect();
  render(session.transcript);
  setStatus(`Connected as user:${session.actorId}`);
}

async function start() {
  if (!root) throw new Error("the session chat UI shell is unavailable");
  const { repositoryId } = await loadUiContext();
  client = new SessionGitClient({ repositoryId, repositoryUrl: `/_heph/git/${repositoryId}` });
  const session = await client.connect();
  render(session.transcript);
  setStatus(`Connected as user:${session.actorId}`);
}

form?.addEventListener("submit", async (event) => {
  event.preventDefault();
  const message = input.value;
  if (!message.trim() || !client?.verifiedActorId) return;
  input.disabled = true;
  setStatus("Publishing message…");
  try {
    await client.appendHuman(makeHumanMessage({ recordId: crypto.randomUUID(), actorId: `user:${client.verifiedActorId}`, text: message }));
    input.value = "";
    const session = await client.readSession();
    render(session.transcript);
    setStatus(`Connected as user:${session.actorId}`);
  } catch (error) {
    setStatus(error instanceof Error ? error.message : "Message could not be published", true);
  } finally {
    input.disabled = false;
    input.focus();
  }
});

reconnect?.addEventListener("click", async () => {
  reconnect.disabled = true;
  setStatus("Reconnecting…");
  try { await refresh(); } catch (error) { setStatus(error instanceof Error ? error.message : "Reconnect failed", true); }
  reconnect.disabled = false;
});

if (root) start().catch((error) => setStatus(error instanceof Error ? error.message : "Session could not be opened", true));
