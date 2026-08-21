// The interface. It holds no keys, no sockets and no files: every line below
// either draws something or calls `invoke`, which crosses `kols-api`.
//
// One rule worth stating because it is the property `design/05` §3 asks for:
// nothing here appends what it just received. A channel is re-read and redrawn
// from what the core returns, so the same record arriving twice — over gossip,
// then inside a segment — is one message rather than two.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const el = (id) => document.getElementById(id);
const state = {
  channels: [],
  current: null,
  me: null,
  // This node's standing with the relay, as last reported. Null means it has
  // not reported yet — at startup, and again after a restart.
  relay: null,
  // The designated set as last drawn, and when a restart was last taken. Both
  // exist to keep the automatic restart below from firing twice for one change.
  designated: null,
  restartedAt: 0,
};

/// How long after a restart to ignore a changed relay set.
///
/// The entry this window writes comes back through replay a moment later, which
/// is the same signal as somebody else designating one. Without this the window
/// would restart for its own action twice.
const RESTART_QUIET_MILLIS = 8000;

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

/// Whether this network is readable yet.
///
/// A member who has joined and not been keyed in holds an identity and nothing
/// else, which is an ordinary place to be rather than a fault — and looks
/// identical to an empty network unless something says so.
function drawKeyState(me) {
  el("key-state").textContent = me.has_key
    ? ""
    : "waiting to be keyed in — you can read nothing here until a member admits you";
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
  el("doorway").hidden = !me.may_invite;
  if (me.may_invite) drawWaiting();

  // Shown to every member, unlike the door: whether this node has a way through
  // NAT is not a privileged question, and a member who cannot fix it still
  // benefits from knowing that is what is wrong.
  el("relay").hidden = false;
  el("relay-set").hidden = !me.may_set_relays;
  el("relay-network-id").textContent = me.network;
  drawRelays();

  drawKeyState(me);
}

// What this network designates, and what this node made of it.
//
// Two lists rather than one, because they disagree in exactly the case that is
// hard to debug: a node whose cache names a relay that is gone behaves
// differently from one that never had a relay, and the difference is invisible
// if only the designated set is shown.
async function drawRelays({ act = false } = {}) {
  let relays;
  try {
    relays = await invoke("relays");
  } catch (err) {
    // Reached by a member who has joined and not been admitted: they hold an
    // identity and are served no state to read a policy out of. The refusal is
    // written in words already, and saying it beats leaving the "…" this starts
    // as, which reads as a request still in flight and never resolves.
    const line = el("relay-state");
    line.className = "relay-state";
    line.textContent = String(err);
    el("relay-list").replaceChildren();
    return;
  }

  // A relay is dialled when a node starts, so a relay this node learned *after*
  // it started is designated and unused. That is how a member who joined before
  // the network had one finds out about it: through replay, while their node has
  // been running since before it existed.
  //
  // Only when there is no circuit, so a working node is never interrupted for a
  // relay it does not need. Not a loop: a restart that fails to get a circuit
  // leaves the set unchanged, so nothing fires again.
  const designated = relays.designated.join(" ");
  const changed = state.designated !== null && state.designated !== designated;
  state.designated = designated;
  if (
    act &&
    changed &&
    !state.relay?.reserved &&
    Date.now() - state.restartedAt > RESTART_QUIET_MILLIS
  ) {
    await restart("this network designated a relay — reconnecting through it");
    return;
  }

  const list = el("relay-list");
  list.replaceChildren();
  for (const address of relays.designated) {
    const row = document.createElement("li");
    row.className = "mono";
    row.textContent = address;
    row.title = address;
    list.append(row);
  }
  for (const address of relays.cached) {
    if (relays.designated.includes(address)) continue;
    const row = document.createElement("li");
    row.className = "mono cached";
    row.textContent = address;
    row.title = "cached locally, and not what replay designates";
    list.append(row);
  }

  drawRelayState(relays);
}

/// One line saying whether this node has a way through NAT.
///
/// The four states `kols serve` distinguishes, kept distinct here. Until now the
/// window had only the bad ones, because relay failures crossed as `Degraded`
/// and success was a `println!` a window never sees — so it could report relay
/// trouble and never relay health.
function drawRelayState(relays) {
  const line = el("relay-state");
  const standing = state.relay;

  if (standing?.reserved) {
    line.className = "relay-state good";
    line.textContent = `reserved a circuit on ${short(standing.reserved)}`;
    return;
  }
  if (relays.designated.length === 0) {
    line.className = "relay-state none";
    line.textContent =
      "none designated — you are reachable only on your own addresses, and " +
      "cannot invite anybody yet";
    return;
  }
  if (standing && standing.designated > 0) {
    line.className = "relay-state bad";
    line.textContent =
      "designated, but none of them granted a circuit — nobody behind a router " +
      "can reach you";
    return;
  }
  // No standing yet: the node reports once, at startup, so this is the gap
  // before it has. Not "broken", which is the wrong thing to say for a second.
  line.className = "relay-state";
  line.textContent = "designated — waiting for this node to report";
}

/// Restarts the node, and says so where the relay's standing is shown.
///
/// The node reports its relay standing once, at startup, so a restart is also
/// the only way to get a fresh answer — which makes the "reconnecting" line
/// honest rather than decorative.
async function restart(why) {
  state.relay = null;
  state.restartedAt = Date.now();
  const line = el("relay-state");
  line.className = "relay-state";
  line.textContent = `${why}…`;
  try {
    await invoke("restart_node");
  } catch (err) {
    line.className = "relay-state bad";
    line.textContent = String(err);
  }
}

/// Enough of an address to recognise, since a full one wraps three lines.
function short(address) {
  const parts = address.split("/p2p/");
  return parts.length === 2 ? `${parts[0]}/…${parts[1].slice(-6)}` : address;
}

// Who is at the door, and letting them in.
//
// The waiting room is live state in the running node, which writes it down for
// anything else to read — so this is stale by construction and says so rather
// than presenting a stale list as the truth.
async function drawWaiting() {
  const waiting = await invoke("waiting");
  const list = el("waiting-list");
  list.replaceChildren();

  const note = el("waiting-note");
  note.hidden = waiting.length !== 0;
  note.textContent =
    "Nobody is waiting. A waiting room only fills while this network is open " +
    "and running, since that is what answers an invite.";

  for (const who of waiting) {
    const row = document.createElement("li");
    const name = document.createElement("span");
    name.className = "mono";
    name.textContent = who.short;

    const button = document.createElement("button");
    button.textContent = "admit";
    button.addEventListener("click", async () => {
      button.disabled = true;
      try {
        await invoke("admit", { identity: who.identity });
        await drawWaiting();
      } catch (err) {
        button.disabled = false;
        el("refused").hidden = false;
        el("refused").textContent = String(err);
      }
    });

    row.append(name, button);
    list.append(row);
  }
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
  // Somebody redeeming an invite is one of the things an event means, and a
  // waiting room nobody redraws is a person standing at a door that never
  // opens.
  if (state.me?.may_invite) await drawWaiting();
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

el("new-invite").addEventListener("click", async () => {
  try {
    const invite = await invoke("create_invite", { uses: 1, hours: 24 });
    el("invite-out").hidden = false;
    el("invite-uri").value = invite.uri;
    // Said rather than assumed: the addresses inside it are the ones this node
    // last reported, so an invite pointing at a node nobody is running connects
    // to nothing.
    el("invite-note").textContent =
      `Good for ${invite.uses} join(s), for about ${invite.hours} more hour(s). ` +
      "It carries this node's addresses, so keep this network open for anybody to redeem it.";
    el("invite-uri").select();
  } catch (err) {
    // The common one is having no relay, which is an ordering problem rather
    // than a fault: a network needs one before it can invite anybody.
    el("invite-out").hidden = false;
    el("invite-uri").value = "";
    el("invite-note").textContent = String(err);
  }
});

el("copy-invite").addEventListener("click", async () => {
  const uri = el("invite-uri").value;
  if (!uri) return;
  await navigator.clipboard.writeText(uri);
  el("copy-invite").textContent = "copied";
  setTimeout(() => (el("copy-invite").textContent = "copy"), 1500);
});

el("copy-network").addEventListener("click", async () => {
  const id = el("relay-network-id").textContent;
  if (!id) return;
  await navigator.clipboard.writeText(id);
  el("copy-network").textContent = "copied";
  setTimeout(() => (el("copy-network").textContent = "copy"), 1500);
});

el("relay-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const relays = el("relay-input").value.trim();
  if (!relays) return;

  const button = event.target.querySelector("button");
  const error = el("relay-error");
  button.disabled = true;
  error.hidden = true;
  try {
    await invoke("set_relays", { relays });
    el("relay-input").value = "";
    // The node was restarted by the command itself, so its standing is unknown
    // again until it reports. Recorded here too, so the same change coming back
    // through replay a moment later is not mistaken for somebody else's.
    state.relay = null;
    state.restartedAt = Date.now();
    await drawRelays();
    error.hidden = false;
    error.className = "relay-error good";
    error.textContent =
      "designated, and this node is reconnecting through it now. Every other " +
      "member learns it by replay.";
  } catch (err) {
    // A refusal is an answer: a malformed address, or a member without
    // define-policy. Both belong here rather than in the channel's refusals.
    error.hidden = false;
    error.className = "relay-error";
    error.textContent = String(err);
  } finally {
    button.disabled = false;
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

el("joiner").addEventListener("submit", async (event) => {
  event.preventDefault();
  const invite = el("invite").value.trim();
  if (!invite) return;

  const button = event.target.querySelector("button");
  button.disabled = true;
  button.textContent = "joining…";
  try {
    const landed = await invoke("join_network", { invite });
    el("invite").value = "";
    state.current = null;
    if (landed.admitted) {
      await start();
      return;
    }
    // Waiting is a successful join, not a failure: an invite to a network that
    // screens its members buys a connection and an identity and nothing else,
    // until somebody admits you. Saying so beats an empty channel list.
    fail(
      `You are in. This network screens its members, so you are waiting to be ` +
        `admitted — ask a member to run:\n\n  kols admit ${landed.identity}`,
    );
    await start();
  } catch (err) {
    fail(err);
  } finally {
    button.disabled = false;
    button.textContent = "join";
  }
});

el("switcher").addEventListener("click", drawPicker);

/// What the node learns, while it runs.
///
/// Every one of these re-reads rather than patching what is on screen. That is
/// `design/05` §3's third property in the smallest form it takes: a record that
/// arrived over gossip is also inside the segment that follows it, so a consumer
/// that appended what it was handed would show every message twice. Re-reading
/// makes a duplicate a non-event, with no bookkeeping to get wrong.
async function watch() {
  await listen("kols://records", async (event) => {
    if (event.payload === state.current) await openChannel(state.current);
  });

  // Channels, permissions and names all come out of replay, so anything that
  // moved the log may have made the sidebar and the header stale.
  await listen("kols://governance", async () => {
    drawMe(await invoke("me"));
    drawChannels(await invoke("channels"));
    // Relays are policy, so an entry that moved the log may have changed them —
    // which is how a member learns of a relay designated after they joined. The
    // only place that passes `act`: a change learned from the log is the case
    // where this node is running without having dialled what the network now
    // names.
    await drawRelays({ act: true });
    if (state.current) await openChannel(state.current);
  });

  await listen("kols://keys", async () => {
    drawMe(await invoke("me"));
  });

  // The node's standing with the relay, reported once at startup — and
  // reported on success, which is the half a window never used to get.
  await listen("kols://relay", async (event) => {
    const [reserved, designated] = event.payload;
    state.relay = { reserved, designated };
    await drawRelays();
  });

  await listen("kols://degraded", (event) => {
    // A node carrying on after something did not work. Shown where the user is
    // looking rather than swallowed: a node quietly failing at one thing looks
    // exactly like a node with nothing to do.
    el("refused").hidden = false;
    el("refused").textContent = String(event.payload);
  });
}

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

watch();
start();
