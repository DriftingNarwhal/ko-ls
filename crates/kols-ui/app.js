// The interface. It holds no keys, no sockets and no files: every line below
// either draws something or calls `invoke`, which crosses `kols-api`.
//
// One rule worth stating because it is the property `design/05` §3 asks for:
// nothing here appends what it just received. A channel is re-read and redrawn
// from what the core returns, so the same record arriving twice — over gossip,
// then inside a segment — is one message rather than two.

const { invoke } = window.__TAURI__.core;

const el = (id) => document.getElementById(id);
const state = { channels: [], current: null, me: null };

/// Which view is showing. There are only two, and no network open is not an
/// error state — it is where somebody starts.
function show(view) {
  document.querySelector(".app").hidden = view !== "app";
  el("picker").hidden = view !== "picker";
}

async function drawPicker() {
  const networks = await invoke("networks");
  const list = el("picker-list");
  list.replaceChildren();

  for (const network of networks) {
    const item = document.createElement("li");
    const button = document.createElement("button");
    button.textContent = network.label || network.id.slice(0, 12);
    button.addEventListener("click", () => openNetwork(network.id));

    const note = document.createElement("span");
    note.className = "picker-note";
    // Not "broken": a network you have joined and not yet been keyed into is a
    // normal place to be, and saying so is better than an empty channel list.
    note.textContent = network.keyed ? "" : "not keyed in yet";

    item.append(button, note);
    list.append(item);
  }

  el("picker-list-wrap").hidden = networks.length === 0;
  show("picker");
}

async function openNetwork(id) {
  try {
    await invoke("open_network", { network: id });
    state.current = null;
    await start();
  } catch (err) {
    fail(err);
  }
}

function fail(err) {
  el("picker-error").hidden = false;
  el("picker-error").textContent = String(err);
}

/// Draws the network header and gates the chrome on what this member holds.
function drawMe(me) {
  state.me = me;
  el("network-label").textContent = me.label || "unnamed network";
  el("network-id").textContent = me.network.slice(0, 16);
  el("you-name").textContent = me.name ?? "unnamed";
  // The id is shown beside the name rather than instead of it: spec 07 §8 makes
  // that an obligation, because uniqueness does not fold confusables and a name
  // alone cannot tell two members apart.
  el("identity").textContent = me.identity;

  // `design/09` §5: controls for actions this member cannot perform are not
  // shown. The hidden control and the refused command are independent, and the
  // second is the one that matters.
  el("new-channel").hidden = !me.may_create_channel;

  // Said plainly rather than left to be inferred from an empty channel: without
  // an epoch key this node can fetch content and open none of it.
  el("key-state").textContent = me.has_key
    ? ""
    : "no epoch key — run `kols serve` to key this network";
}

function drawChannels(channels) {
  state.channels = channels;
  const list = el("channel-list");
  list.replaceChildren();

  for (const channel of channels) {
    const item = document.createElement("li");
    const button = document.createElement("button");
    button.textContent = `#${channel.name}`;
    button.dataset.id = channel.id;
    button.className = channel.id === state.current ? "current" : "";
    if (channel.private) button.title = "private";
    if (channel.archived) button.classList.add("archived");
    button.addEventListener("click", () => openChannel(channel.id));
    item.append(button);
    list.append(item);
  }

  if (channels.length === 0) {
    const empty = document.createElement("li");
    empty.className = "empty";
    empty.textContent = "no channels yet";
    list.append(empty);
  }
}

function drawMessages(opened) {
  const channel = state.channels.find((c) => c.id === opened.channel);
  el("channel-name").textContent = channel ? `#${channel.name}` : "channel";
  el("channel-topic").textContent = channel ? channel.topic : "";

  const list = el("messages");
  list.replaceChildren();

  for (const message of opened.messages) {
    const row = document.createElement("li");
    row.className = "message";
    row.dataset.kols = "message";
    if (message.withdrawn) row.classList.add("withdrawn");
    if (message.redacted) row.classList.add("redacted");
    if (message.pinned) row.classList.add("pinned");

    const who = document.createElement("span");
    who.className = "author";
    who.textContent = message.author;
    who.title = message.author_id;

    const whoId = document.createElement("span");
    whoId.className = "author-id";
    // Only where a name is standing in for the id, since a member with no name
    // is already shown as one and repeating it says nothing.
    whoId.textContent = message.author === message.author_id ? "" : message.author_id;

    const at = document.createElement("span");
    at.className = "at";
    at.textContent = message.at;

    const body = document.createElement("span");
    body.className = "body";
    // Withdrawn and redacted mean *hidden*, never unsent — `design/01` §6. The
    // interface says which happened rather than pretending the message was
    // never written.
    body.textContent = message.withdrawn
      ? "(withdrawn by its author)"
      : message.redacted
        ? "(hidden by a moderator)"
        : message.body;

    row.append(at, who, whoId, body);

    if (message.edited && !message.withdrawn) {
      const edited = document.createElement("span");
      edited.className = "flag";
      edited.textContent = "edited";
      row.append(edited);
    }

    for (const reaction of message.reactions) {
      const chip = document.createElement("span");
      chip.className = "reaction";
      chip.textContent = `${reaction.key} ${reaction.count}`;
      row.append(chip);
    }

    list.append(row);
  }

  list.scrollTop = list.scrollHeight;

  // A record this node refused is one another client may be showing. Silence
  // would make the two look like they agree.
  const refused = el("refused");
  refused.hidden = opened.refused.length === 0;
  refused.textContent = opened.refused.length
    ? `refused ${opened.refused.length} record(s): ${opened.refused.join(", ")}`
    : "";
}

async function openChannel(id) {
  state.current = id;
  drawChannels(state.channels);

  const opened = await invoke("open_channel", { channel: id });
  drawMessages(opened);

  // Three states, and they are genuinely different: you may not post here, you
  // have not claimed a name yet, or you can write.
  const mayPost = state.me?.may_post ?? false;
  const named = Boolean(state.me?.name);
  el("composer").hidden = !mayPost || !named;
  el("composer-denied").hidden = mayPost;
  el("namer").hidden = !mayPost || named;
}

async function refresh() {
  drawChannels(await invoke("channels"));
  if (state.current) {
    // Re-read rather than patch: the projection is the core's, and redrawing
    // from it is what makes a duplicate delivery a non-event.
    drawMessages(await invoke("open_channel", { channel: state.current }));
  }
}

el("composer").addEventListener("submit", async (event) => {
  event.preventDefault();
  const body = el("body").value.trim();
  if (!body) return;
  try {
    await invoke("send_message", { channel: state.current, body });
    el("body").value = "";
    await openChannel(state.current);
  } catch (err) {
    // A refusal is an answer, not a crash: too fast, too long, not permitted.
    // It belongs where the user is looking.
    el("refused").hidden = false;
    el("refused").textContent = String(err);
  }
});

el("namer").addEventListener("submit", async (event) => {
  event.preventDefault();
  const name = el("name").value.trim();
  if (!name) return;
  try {
    await invoke("set_name", { name });
    drawMe(await invoke("me"));
    if (state.current) await openChannel(state.current);
  } catch (err) {
    // "that name is held by …" is the common one, and it is an answer rather
    // than a fault: pick another.
    el("refused").hidden = false;
    el("refused").textContent = String(err);
  }
});

el("new-channel").addEventListener("click", async () => {
  const name = prompt("channel name");
  if (!name) return;
  try {
    await invoke("create_channel", { name, topic: "" });
    await refresh();
  } catch (err) {
    el("refused").hidden = false;
    el("refused").textContent = String(err);
  }
});

el("maker").addEventListener("submit", async (event) => {
  event.preventDefault();
  const name = el("new-name").value.trim();
  const relay = el("new-relay").value.trim();
  if (!name) return;
  try {
    await invoke("create_network", { name, relay });
    el("new-name").value = "";
    el("new-relay").value = "";
    state.current = null;
    await start();
  } catch (err) {
    fail(err);
  }
});

el("switcher").addEventListener("click", drawPicker);

async function start() {
  let me;
  try {
    me = await invoke("me");
  } catch {
    // No network open. The first thing this client asks is which one, and with
    // none it asks whether to make one.
    await drawPicker();
    return;
  }

  show("app");
  drawMe(me);
  const channels = await invoke("channels");
  drawChannels(channels);
  if (channels.length > 0) await openChannel(channels[0].id);
}

start();
