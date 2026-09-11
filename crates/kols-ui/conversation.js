// A conversation window — `design/09` §1.6, D40.
//
// One person, one implied channel, and nothing else. It holds no keys, no
// sockets and no files: every line here either draws something or calls
// `invoke`, which crosses `kols-api`.
//
// # What it deliberately does not have
//
// No channel rail, no roster dropdown, no settings screen. A
// `conversation`-profile network has exactly one channel and no roles (spec 07
// §1.2), so drawing those would be furniture that is always empty and controls
// that cannot exist. Nothing here is a smaller version of the network window;
// it is a different window because a conversation is a different thing.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const el = (id) => document.getElementById(id);

/// Which conversation this window is for, from the URL the shell opened it at.
///
/// Carried in the address rather than asked for, so that several of these can
/// be open at once and each knows its own — the shell's single "open network"
/// is not an answer here (`09` §1.6: several at once is the shape's advantage).
const network = new URLSearchParams(window.location.search).get("network") ?? "";

/// How often to re-read. Same two seconds the network window uses.
const TICK_MILLIS = 2000;

let lastDrawn = "";

function fail(err) {
  const line = el("error");
  line.hidden = false;
  line.textContent = String(err && err.message ? err.message : err);
}

/// Who this is with, and whether it can still be reached.
async function drawWho() {
  let who;
  try {
    who = await invoke("conversation_who", { network });
  } catch (err) {
    fail(err);
    return;
  }
  el("who").textContent = who.label || (who.who ? who.who.slice(0, 8) : "a conversation");

  // **A name is never sufficient on its own** (spec 07 §8): the uniqueness key
  // does not fold confusables, so the identity goes beside the name wherever
  // somebody is deciding who they are talking to — and that is this window.
  el("where").textContent = who.who
    ? `${who.who.slice(0, 8)} · met in ${who.shared ? who.shared.slice(0, 8) : "a network you shared"}`
    : "";

  // D39's owed sentence, said where it is true rather than rendered as silence
  // (`09` §1.9). A conversation borrows its rendezvous from the network it was
  // arranged in, on a permission recomputed every time — so when that lapses
  // it stops connecting, and stopping connecting looks exactly like nobody
  // talking.
  const stopped = el("stopped");
  if (who.state === "adrift") {
    stopped.hidden = false;
    stopped.textContent =
      "This machine has nowhere to meet them: you no longer share a network, " +
      "so nothing new can cross. What is already here stays readable.";
  } else {
    stopped.hidden = true;
  }
}

/// Draws the messages.
///
/// Re-read and redrawn from what the core returns rather than appended to —
/// `05` §3's third property. A record arriving over gossip is *also* inside the
/// segment that follows it, so duplicate delivery is the normal case and a
/// consumer that appended what it was handed would show every message twice.
async function drawMessages() {
  let opened;
  try {
    opened = await invoke("conversation_read", { network });
  } catch (err) {
    fail(err);
    return;
  }
  el("error").hidden = true;

  // Nothing is redrawn while nothing has changed, so a two-second tick does not
  // fight somebody selecting text.
  const fingerprint = JSON.stringify(opened.messages.map((m) => [m.id, m.body, m.withdrawn]));
  if (fingerprint === lastDrawn) return;
  lastDrawn = fingerprint;

  const list = el("messages");
  const atBottom =
    list.scrollHeight - list.scrollTop - list.clientHeight < 40 || list.scrollTop === 0;
  list.replaceChildren();

  for (const message of opened.messages) {
    const row = document.createElement("div");
    row.className = "said";
    if (message.mine) row.classList.add("mine");

    const body = document.createElement("p");
    body.className = "said-body";
    // **Withdrawn is not deleted, and the interface must not imply it is**
    // (`01` §6, `05` §5): it stops conformant clients rendering a message and
    // retracts no bytes anybody already has.
    body.textContent = message.withdrawn ? "withdrawn" : message.body;
    if (message.withdrawn) body.classList.add("withdrawn");

    const when = document.createElement("span");
    when.className = "said-when";
    when.textContent = message.at;

    row.append(body, when);
    list.append(row);
  }

  if (atBottom) list.scrollTop = list.scrollHeight;
}

el("composer").addEventListener("submit", async (event) => {
  event.preventDefault();
  const body = el("body").value.trim();
  if (!body) return;
  try {
    await invoke("conversation_send", { network, body });
    el("body").value = "";
    lastDrawn = "";
    await drawMessages();
  } catch (err) {
    fail(err);
  }
});

// Only what arrived in *this* conversation. Several nodes run at once
// (`09` §2), so an event carries the network it came from and one from
// anywhere else must not redraw what is on screen.
listen("kols://records", async (event) => {
  const from = Array.isArray(event.payload) ? event.payload[0] : event.payload;
  if (from && network && !network.startsWith(from) && !from.startsWith(network)) return;
  lastDrawn = "";
  await drawMessages();
});

async function start() {
  if (!network) {
    fail("this window was opened without a conversation");
    return;
  }
  await drawWho();
  await drawMessages();
  setInterval(() => {
    void drawWho();
    void drawMessages();
  }, TICK_MILLIS);
}

start();
