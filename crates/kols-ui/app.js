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
  // The sidebar as the network orders it — spec 07 §1.6, computed in the core
  // and drawn here. `channels` is this flattened, for the lookups that want it.
  sidebar: [],
  // The channel being dragged, or null. Only ever set where the capability to
  // move one is held.
  dragging: null,
  mayManage: false,
  current: null,
  me: null,
  // The designated set as last drawn, and when a restart was last taken. Both
  // exist to keep the automatic restart below from firing twice for one change.
  designated: null,
  restartedAt: 0,
  relayPoll: null,
  doorPoll: null,
  channelPoll: null,
  // Channel id to how many messages have arrived there unseen.
  unread: {},
  // What `me` and the channel list looked like when last drawn, so a tick that
  // finds nothing new leaves the sidebar alone.
  meSignature: null,
  sidebarSignature: null,
  peopleSignature: null,
  // What the open channel looked like when it was last drawn, so a poll that
  // finds nothing new does no DOM work.
  channelSignature: null,
  // Which settings panel is showing, and which role is expanded in it. Both are
  // local view state that reaches nobody: `design/09` §4.2's line runs between
  // what a *change* costs, and looking at a panel costs nothing.
  settingsTab: "network",
  role: null,
};

/// How often to re-read the open channel.
///
/// Matches the daemon's own 2-second sync tick: asking faster than records can
/// arrive only costs replays.
const CHANNEL_REFRESH_MILLIS = 2000;

/// How often to re-read the waiting room while it is on screen.
///
/// Somebody is standing at a door waiting to be let in, so this is the interval
/// at which the person who can let them in finds out. Cheap: a local file the
/// node has already written.
const DOOR_REFRESH_MILLIS = 4000;

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
  state.meSignature = signatureOfMe(me);
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
  el("new-folder").hidden = !me.may_manage_channel;
  state.mayManage = me.may_manage_channel;
  el("doorway").hidden = !me.may_invite;
  if (me.may_invite) drawWaiting();

  // Shown to every member, unlike the door: whether this node has a way through
  // NAT is not a privileged question, and a member who cannot fix it still
  // benefits from knowing that is what is wrong.
  el("roster").hidden = false;
  drawPeople();

  // Re-read on a timer as well as on the event. `design/09` §4 already calls the
  // waiting room stale by construction — it is live state in the node, written
  // down for anything else to read — so refreshing it is the model rather than a
  // patch over one. It is a local file read, and only while somebody is looking
  // at a door they can actually open.
  watchDoor(me.may_invite);

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
    !relays.reserved &&
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

  if (relays.reserved) {
    line.className = "relay-state good";
    line.textContent = `reserved a circuit on ${short(relays.reserved)}`;
    stopWatchingRelay();
    return;
  }
  if (relays.designated.length === 0) {
    line.className = "relay-state none";
    line.textContent =
      "none designated — you are reachable only on your own addresses, and " +
      "cannot invite anybody yet";
    stopWatchingRelay();
    return;
  }
  if (relays.reported) {
    line.className = "relay-state bad";
    // The reasons rather than a summary of them. "No circuit" has two causes
    // needing opposite fixes — nothing answered there, or something answered
    // and named no address — and only the node knows which happened.
    line.textContent =
      relays.failures.length > 0
        ? relays.failures.join(" · ")
        : "designated, and no circuit was granted";
    // The note is about a relay that answered. It says nothing useful about one
    // nothing could reach, so it is shown only for the case it explains.
    el("relay-help").hidden = !relays.failures.some((why) =>
      why.includes("no circuit from"),
    );
    stopWatchingRelay();
    return;
  }
  // Not reported yet. The node settles this within about 20 seconds of
  // starting, so this is a real "asking", and it says so in those words rather
  // than in words that could be mistaken for a verdict.
  line.className = "relay-state working";
  line.textContent = "asking the relay for a circuit…";
}

/// Polls while the node has not reported.
///
/// Belt and braces on the event, which is prompt and can be missed — a node that
/// settles before the webview has registered its listeners emits into nothing.
/// Reservation is bounded at roughly 20 seconds, so this asks a little past that
/// and stops.
function watchRelay() {
  stopWatchingRelay();
  let asked = 0;
  state.relayPoll = setInterval(async () => {
    asked += 1;
    if (asked > 15) {
      stopWatchingRelay();
      return;
    }
    await drawRelays();
  }, 2000);
}

function stopWatchingRelay() {
  if (state.relayPoll) {
    clearInterval(state.relayPoll);
    state.relayPoll = null;
  }
}

/// Restarts the node, and says so where the relay's standing is shown.
///
/// The node settles its relay standing once, when it starts, so restarting is
/// also the only way to get a fresh answer — which makes the "reconnecting" line
/// honest rather than decorative.
async function restart(why) {
  state.restartedAt = Date.now();
  const line = el("relay-state");
  line.className = "relay-state working";
  line.textContent = `${why}…`;
  el("relay-help").hidden = true;
  try {
    await invoke("restart_node");
    watchRelay();
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

/// Keeps the waiting room fresh while a member who can admit is looking at it.
function watchDoor(may_invite) {
  if (state.doorPoll) {
    clearInterval(state.doorPoll);
    state.doorPoll = null;
  }
  if (!may_invite) return;
  state.doorPoll = setInterval(drawWaiting, DOOR_REFRESH_MILLIS);
}

/// Who is in this network, and which of them this node is talking to.
///
/// The dot means **connected to this node**, and the note under the list says
/// so. It is not presence: there is no routing (Core §5.2) and this client dials
/// the peers it has addresses for rather than every member, so an unlit member
/// may be away, unreachable from here, or simply never dialled — and nothing
/// here can tell those apart. Saying "online" would be the kind of wrong that
/// gets worse as a network grows.
async function drawPeople() {
  let people;
  try {
    people = await invoke("people");
  } catch {
    return;
  }

  const signature = people
    .map((p) => `${p.identity}|${p.name ?? ""}|${p.connected}`)
    .join(",");
  if (signature === state.peopleSignature) return;
  state.peopleSignature = signature;

  const list = el("roster-list");
  list.replaceChildren();
  for (const person of people) {
    const row = document.createElement("li");
    row.className = person.connected ? "person connected" : "person";

    const dot = document.createElement("span");
    dot.className = "dot";
    dot.title = person.you
      ? "you"
      : person.connected
        ? "connected to you right now"
        : "not connected to you — away, unreachable from here, or never dialled";

    const who = document.createElement("span");
    who.className = "who";
    who.textContent = person.name ?? person.short;
    if (person.you) who.textContent += " (you)";

    // Spec 07 §8: a name never stands in for an identity, because uniqueness is
    // decided on a key that does not fold confusables.
    const id = document.createElement("span");
    id.className = "mono person-id";
    id.textContent = person.short;

    row.append(dot, who, id);
    list.append(row);
  }

  const live = people.filter((person) => person.connected || person.you).length;
  el("roster-count").textContent = `${live}/${people.length}`;
  el("roster-note").textContent =
    "A lit dot means connected to you right now. An unlit one means away, " +
    "unreachable from here, or never dialled — this client cannot tell those apart.";
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

/// The sidebar, in the order the network agrees on — spec 07 §1.6.
///
/// **The order arrives already computed.** That default is normative — every
/// node must reach the same one — and `kols_core::sidebar_order` is its tested
/// implementation, so sorting here would put a second implementation of a rule
/// two members must never disagree about right next to the first. This draws
/// what it is given, in the order it is given.
function drawSidebar(rows) {
  state.sidebar = rows;
  state.channels = flattenChannels(rows);
  state.sidebarSignature = signatureOfSidebar(rows);
  const list = el("channel-list");
  list.replaceChildren();

  for (const row of rows) {
    if (row.kind === "channel") {
      list.append(channelItem(row.channel, null));
    } else {
      list.append(folderItem(row));
    }
  }

  if (state.channels.length === 0 && rows.length === 0) {
    const empty = document.createElement("li");
    empty.className = "empty";
    empty.textContent = "no channels yet";
    list.append(empty);
  }
}

/// Every channel, folders flattened away, for the callers that want a lookup.
function flattenChannels(rows) {
  const out = [];
  for (const row of rows) {
    if (row.kind === "channel") out.push(row.channel);
    else out.push(...row.channels);
  }
  return out;
}

/// One channel row. `category` is the folder it sits in, or null at top level.
function channelItem(channel, category) {
  const item = document.createElement("li");
  item.className = "channel-item";
  item.dataset.channel = channel.id;
  item.dataset.category = category ?? "";

  const button = document.createElement("button");
  button.dataset.id = channel.id;
  button.className = channel.id === state.current ? "current" : "";
  if (channel.private) button.title = "private";
  if (channel.archived) button.classList.add("archived");

  const name = document.createElement("span");
  name.className = "channel-name";
  name.textContent = `#${channel.name}`;
  button.append(name);

  // Two signals for one fact, because they answer different questions from
  // different distances: the weight says *something is here* at a glance, and
  // the count says how much once you look.
  const unread = state.unread[channel.id] ?? 0;
  if (unread > 0 && channel.id !== state.current) {
    button.classList.add("unread");
    const badge = document.createElement("span");
    badge.className = "unread-count";
    badge.textContent = unread > 99 ? "99+" : String(unread);
    button.append(badge);
  }

  button.addEventListener("click", () => openChannel(channel.id));

  // Gated on the capability, like every other control here. Hiding is
  // presentation only — the command is re-checked on receipt regardless.
  if (state.mayManage) {
    button.draggable = true;
    button.addEventListener("dragstart", (event) => {
      state.dragging = { channel: channel.id };
      event.dataTransfer.effectAllowed = "move";
      // Firefox will not start a drag without data set.
      event.dataTransfer.setData("text/plain", channel.id);
    });
    button.addEventListener("dragend", () => {
      state.dragging = null;
      clearDropMarks();
    });
    button.addEventListener("contextmenu", (event) =>
      channelMenu(event, channel, category),
    );
  }

  item.append(button);
  wireDrop(item, category);
  return item;
}

/// One folder, and the channels inside it.
function folderItem(row) {
  const item = document.createElement("li");
  item.className = "folder";
  item.dataset.category = row.id;

  const head = document.createElement("div");
  head.className = "folder-head";

  const label = document.createElement("span");
  label.className = "folder-name";
  // A channel may name a category nothing ever defined (spec 07 §1.8). That is
  // not an error and not a blank: saying so is better than an empty row nobody
  // can explain.
  label.textContent = row.name || "unnamed folder";
  if (!row.name) label.classList.add("unnamed");
  head.append(label);

  if (state.mayManage) {
    head.addEventListener("contextmenu", (event) => folderMenu(event, row));
    head.title = "right-click for folder actions";
  }
  item.append(head);

  const inner = document.createElement("ul");
  inner.className = "folder-channels";
  for (const channel of row.channels) inner.append(channelItem(channel, row.id));
  if (row.channels.length === 0) {
    const empty = document.createElement("li");
    empty.className = "empty";
    empty.textContent = "empty";
    inner.append(empty);
  }
  item.append(inner);

  // The whole folder is a drop target, so dropping onto its header or its empty
  // space puts a channel at the end of it.
  wireDrop(item, row.id, true);
  return item;
}

/// Enough of the sidebar to tell whether it would draw differently.
function signatureOfSidebar(rows) {
  return rows
    .map((row) =>
      row.kind === "channel"
        ? `c:${row.channel.id}|${row.channel.name}|${row.channel.topic}|${row.channel.archived}`
        : `f:${row.id}|${row.name}|` +
          row.channels
            .map((c) => `${c.id}|${c.name}|${c.topic}|${c.archived}`)
            .join("~"),
    )
    .join(",");
}

// ── moving things ──────────────────────────────────────────────────────

function clearDropMarks() {
  for (const node of document.querySelectorAll(".drop-into, .drop-before")) {
    node.classList.remove("drop-into", "drop-before");
  }
}

/// Makes a node accept a dropped channel. `into` marks a folder body rather than
/// a row, which means "at the end of this folder" instead of "before this row".
function wireDrop(node, category, into = false) {
  node.addEventListener("dragover", (event) => {
    if (!state.dragging) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "move";
    clearDropMarks();
    node.classList.add(into ? "drop-into" : "drop-before");
  });
  node.addEventListener("drop", async (event) => {
    if (!state.dragging) return;
    event.preventDefault();
    event.stopPropagation();
    const moved = state.dragging.channel;
    state.dragging = null;
    clearDropMarks();
    const before = into ? null : node.dataset.channel;
    await moveChannelTo(moved, category, before);
  });
}

/// The channels that would be siblings in `category`, in their drawn order.
function siblingsIn(category) {
  if (category === null || category === "") {
    return state.sidebar
      .filter((row) => row.kind === "channel")
      .map((row) => row.channel);
  }
  const folder = state.sidebar.find(
    (row) => row.kind === "category" && row.id === category,
  );
  return folder ? folder.channels : [];
}

/// Moves `channel` into `category`, landing before `before` or at the end.
///
/// **Positions are chosen sparsely and split at the midpoint**, so an ordinary
/// drag writes one governance entry rather than renumbering the folder. When
/// there is no room between two neighbours — or when a sibling has never been
/// positioned, which sorts it last and leaves nothing to measure against — the
/// folder is spaced out once and the drag retried. That pass costs an entry per
/// channel, and it happens once per folder rather than once per drag.
async function moveChannelTo(channel, category, before) {
  const siblings = siblingsIn(category).filter((c) => c.id !== channel);
  const index =
    before === null || before === undefined
      ? siblings.length
      : Math.max(
          0,
          siblings.findIndex((c) => c.id === before),
        );

  const at = index < siblings.length ? index : siblings.length;
  const unpositioned = siblings.some((c) => c.position === null);
  let position = null;

  if (!unpositioned) {
    const lo = at > 0 ? siblings[at - 1].position : null;
    const hi = at < siblings.length ? siblings[at].position : null;
    if (lo === null && hi === null) position = 1024;
    else if (lo === null) position = hi > 0 ? Math.floor(hi / 2) : null;
    else if (hi === null) position = lo + 1024;
    else {
      const mid = Math.floor((lo + hi) / 2);
      position = mid > lo && mid < hi ? mid : null;
    }
  }

  await act(async () => {
    if (position === null) {
      // Space the folder out, then place into the gap we just guaranteed.
      let next = 0;
      for (const sibling of siblings) {
        await invoke("move_channel", {
          channel: sibling.id,
          category: category ?? null,
          position: next,
        });
        next += 1024;
      }
      position = at * 1024 + (at < siblings.length ? 512 : 0);
    }
    await invoke("move_channel", {
      channel,
      category: category ?? null,
      position,
    });
    await refresh();
  });
}

// ── menus ──────────────────────────────────────────────────────────────

/// A small menu at the pointer. Closes on the next click anywhere.
function popMenu(event, entries) {
  event.preventDefault();
  document.querySelector(".pop-menu")?.remove();

  const menu = document.createElement("div");
  menu.className = "pop-menu";
  menu.style.left = `${event.clientX}px`;
  menu.style.top = `${event.clientY}px`;

  for (const [label, run] of entries) {
    const button = document.createElement("button");
    button.textContent = label;
    button.addEventListener("click", async () => {
      menu.remove();
      await run();
    });
    menu.append(button);
  }

  document.body.append(menu);
  setTimeout(() => {
    document.addEventListener("click", () => menu.remove(), { once: true });
  }, 0);
}

/// Turns a sidebar row into a text field, in place.
///
/// Renaming is data entry rather than authorisation, which is why it is allowed
/// to live in the document at all: `design/09` §6.5 keeps anything that asks a
/// member to *authorise* something in native chrome, outside what a theme can
/// reach, and the destructive confirmations below still do. Editing the name you
/// are about to sign is not that, and a modal for it was worse than the thing it
/// was protecting.
///
/// Committed on Enter or on blur, abandoned on Escape. Blur commits rather than
/// cancels because the alternative loses typing to a stray click, and a rename
/// is recoverable by renaming again.
function renameInPlace(node, current, commit) {
  const input = document.createElement("input");
  input.type = "text";
  input.className = "rename";
  input.value = current;
  input.setAttribute("aria-label", "new name");

  let done = false;
  const finish = async (save) => {
    if (done) return;
    done = true;
    const name = input.value.trim();
    input.replaceWith(node);
    if (!save || !name || name === current) return;
    await act(async () => {
      await commit(name);
      await refresh();
    });
  };

  input.addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      void finish(true);
    }
    if (event.key === "Escape") {
      event.preventDefault();
      void finish(false);
    }
  });
  input.addEventListener("blur", () => void finish(true));

  node.replaceWith(input);
  input.focus();
  input.select();
}

function channelMenu(event, channel, category) {
  const folders = state.sidebar.filter((row) => row.kind === "category");
  const row = event.currentTarget;
  const entries = [
    [
      "rename",
      async () => {
        const label = row.querySelector(".channel-name") ?? row;
        renameInPlace(label, channel.name, (name) =>
          invoke("rename_channel", { channel: channel.id, name }),
        );
      },
    ],
    [
      "set topic",
      async () => {
        const topic = await askFor(`Topic for #${channel.name}`, {
          value: channel.topic ?? "",
          placeholder: "what this channel is for",
        });
        // Cancelling is not clearing: null is the member deciding not to, and
        // an empty topic is a topic somebody chose to remove.
        if (topic === null) return;
        await act(async () => {
          await invoke("set_channel_topic", { channel: channel.id, topic });
          await refresh();
        });
      },
    ],
    [
      channel.archived ? "archived" : "archive",
      async () => {
        if (channel.archived) return;
        await act(async () => {
          await invoke("archive_channel", { channel: channel.id });
          await refresh();
        });
      },
    ],
  ];

  if (category !== null && category !== "") {
    entries.push([
      "move out of folder",
      async () => moveChannelTo(channel.id, null, null),
    ]);
  }
  for (const folder of folders) {
    if (folder.id === category) continue;
    entries.push([
      `move to ${folder.name || "unnamed folder"}`,
      async () => moveChannelTo(channel.id, folder.id, null),
    ]);
  }

  entries.push([
    "delete",
    async () => {
      // `01` §6: hidden, not erased. The confirmation says so rather than
      // implying this reaches anything anybody already fetched.
      if (
        !confirm(
          `Delete #${channel.name}?\n\nIt is hidden from listings, not erased — records already fetched stay fetched.`,
        )
      )
        return;
      await act(async () => {
        await invoke("delete_channel", { channel: channel.id });
        await refresh();
      });
    },
  ]);

  popMenu(event, entries);
}

/// Moves a folder one place up or down.
///
/// Positions are rewritten only for the two folders that swap, so reordering
/// costs two governance entries rather than renumbering the sidebar. Where a
/// neighbour has never been positioned there is nothing to swap with, so both
/// ends of the pair are given spaced positions instead.
async function nudgeFolder(row, direction) {
  const folders = state.sidebar.filter((r) => r.kind === "category");
  const at = folders.findIndex((r) => r.id === row.id);
  const to = at + direction;
  if (at < 0 || to < 0 || to >= folders.length) return;

  const other = folders[to];
  const mine = row.position;
  const theirs = other.position;

  await act(async () => {
    if (mine === null || theirs === null) {
      let next = 0;
      const reordered = folders.slice();
      reordered.splice(at, 1);
      reordered.splice(to, 0, row);
      for (const folder of reordered) {
        await invoke("move_category", { category: folder.id, position: next });
        next += 1024;
      }
    } else {
      await invoke("move_category", { category: row.id, position: theirs });
      await invoke("move_category", { category: other.id, position: mine });
    }
    await refresh();
  });
}

function folderMenu(event, row) {
  const head = event.currentTarget;
  popMenu(event, [
    ["move up", async () => nudgeFolder(row, -1)],
    ["move down", async () => nudgeFolder(row, 1)],
    [
      "rename",
      async () => {
        const label = head.querySelector(".folder-name") ?? head;
        renameInPlace(label, row.name ?? "", (name) =>
          invoke("rename_category", { category: row.id, name }),
        );
      },
    ],
    [
      "delete folder",
      async () => {
        // Spec 07 §1.8: this removes a name and a sort key, never a scope. The
        // channels stay in it and resolve exactly what they did before, so the
        // wording must not suggest they are being deleted or moved.
        if (
          !confirm(
            `Delete the folder "${row.name || "unnamed folder"}"?\n\nIts ${row.channels.length} channel(s) keep their permissions and stay grouped — the folder just loses its name.`,
          )
        )
          return;
        await act(async () => {
          await invoke("delete_category", { category: row.id });
          await refresh();
        });
      },
    ],
  ]);
}

/// What a healed fork undid — Core §2.7.1 point 5.
///
/// **Asked for rather than only listened for.** Replay follows the winning
/// branch, so an action that lost leaves no trace in the projection: if this
/// only rendered a pushed event, a window that was not listening yet would never
/// learn that a revocation had been undone. The node holds the last report and
/// this asks for it.
async function drawReorg() {
  let report = null;
  try {
    report = await invoke("reorg");
  } catch {
    return;
  }
  if (!report) return;

  const risky = report.mine.filter((action) => action.security_relevant);
  const box = el("reorg");
  box.hidden = false;
  box.classList.toggle("severe", risky.length > 0);

  const mine = report.mine.length;
  el("reorg-text").textContent =
    mine === 0
      ? `A partition healed. ${report.others} action(s) by other members were undone.`
      : `A partition healed and ${mine} of your action(s) were undone` +
        (report.others > 0 ? `, along with ${report.others} of other members'.` : ".");

  const list = el("reorg-list");
  list.replaceChildren();
  for (const action of risky) {
    const item = document.createElement("li");
    // Named plainly. "Voided" is the log's word; what a person needs to know is
    // that whatever this removed is back until they do it again.
    item.textContent = `${action.kind} was undone — whatever it removed is in effect again until you repeat it.`;
    list.append(item);
  }
}

/// Runs a command and redraws, putting a refusal where the user is looking.
///
/// Every one of these is authorized on receipt, so a refusal here is an answer —
/// too fast, not permitted, not yours to revise — rather than a fault. The
/// redraw is unconditional because the projection is the authority on what
/// happened, not this function's idea of what it asked for.
async function act(run) {
  try {
    await run();
    el("refused").hidden = true;
    if (state.current) await openChannel(state.current);
  } catch (err) {
    el("refused").hidden = false;
    el("refused").textContent = String(err);
  }
}

/// The two reaction keys this client offers as a vote.
///
/// A vote is not a new record kind: spec 07's `Reaction { target, key, remove }`
/// carries a free-form key, so up and down are two of them and the rest of the
/// protocol is unchanged. Another client writing `:tada:` is still conformant,
/// and still rendered here as a chip.
const UP = "+1";
const DOWN = "-1";

/// Up, score, down — one control, because they are one decision.
///
/// Mutually exclusive, which is the whole difference from a reaction: voting up
/// while holding a down vote withdraws the down vote first. That is two records
/// rather than one, both authored by this member, because the log has no notion
/// of changing your mind — only of what you have said.
function votes(channel, message) {
  const box = document.createElement("span");
  box.className = "votes";

  const held = (key) => message.reactions.find((r) => r.key === key);
  const up = held(UP);
  const down = held(DOWN);
  const score = (up?.count ?? 0) - (down?.count ?? 0);

  const cast = (key, mine, opposite) =>
    act(async () => {
      if (mine) {
        await invoke("react", { channel, message: message.id, key, remove: true });
        return;
      }
      if (opposite?.mine) {
        await invoke("react", {
          channel,
          message: message.id,
          key: key === UP ? DOWN : UP,
          remove: true,
        });
      }
      await invoke("react", { channel, message: message.id, key, remove: false });
    });

  const arrow = (label, key, mine, opposite, title) => {
    const button = document.createElement("button");
    button.className = mine ? "vote cast" : "vote";
    button.textContent = label;
    button.title = title;
    button.addEventListener("click", () => cast(key, mine, opposite));
    return button;
  };

  const count = document.createElement("span");
  // Zero is shown rather than hidden: a message nobody voted on and a message
  // with two up and two down are different facts about the same number.
  count.className = score > 0 ? "score up" : score < 0 ? "score down" : "score";
  count.textContent = String(score);
  count.title = `${up?.count ?? 0} up, ${down?.count ?? 0} down`;

  box.append(
    arrow("▲", UP, Boolean(up?.mine), down, up?.mine ? "take back your vote" : "vote up"),
    count,
    arrow("▼", DOWN, Boolean(down?.mine), up, down?.mine ? "take back your vote" : "vote down"),
  );
  return box;
}

/// What this member may do to one message.
///
/// Revising and withdrawing are offered on a member's own messages only, which
/// is spec 07 §5.2's rule shown rather than enforced — the gate refuses either
/// way. Pinning follows `may_moderate`, which is the network-wide capability and
/// misses a per-channel moderator; that error hides a control somebody holds
/// rather than offering one that will be refused.
function actions(channel, message) {
  const bar = document.createElement("span");
  bar.className = "actions";

  const button = (label, title, run) => {
    const it = document.createElement("button");
    it.textContent = label;
    it.title = title;
    it.addEventListener("click", () => act(run));
    bar.append(it);
  };

  if (message.mine && !message.withdrawn) {
    button("edit", "revise this", async () => {
      const body = await askFor("Revise this message", { value: message.body });
      // Distinguished deliberately: cancelling is not the same as clearing, and
      // an empty edit is refused by the gate rather than silently dropped here.
      if (body === null) return;
      await invoke("edit_message", { channel, message: message.id, body });
    });
    button("withdraw", "hidden, not unsent — everybody who has it keeps it", () =>
      confirm("Withdraw this message?\n\nIt is hidden, not unsent: anybody who already has it keeps the bytes.")
        ? invoke("delete_message", { channel, message: message.id })
        : Promise.resolve(),
    );
  }

  if (state.me?.may_moderate) {
    button(message.pinned ? "unpin" : "pin", "needs chat:moderate", () =>
      invoke("pin", { channel, message: message.id, remove: message.pinned }),
    );
  }

  return bar;
}

/// Enough of a channel to tell whether redrawing it would change anything.
function signatureOf(opened) {
  const last = opened.messages.at(-1);
  return [
    opened.channel,
    opened.messages.length,
    last?.id ?? "",
    // Edits, withdrawals, reactions and pins all change a message without
    // changing how many there are, so the last one's shape rides along.
    last ? `${last.body}|${last.edited}|${last.withdrawn}|${last.reactions.length}` : "",
    opened.refused.length,
    // Pins, anywhere in the channel rather than only on the last message: a
    // moderator pinning something from an hour ago changes nothing else about
    // the view, so without this a pin by somebody else would never be drawn.
    opened.messages
      .map((message, index) => (message.pinned ? index : ""))
      .filter((index) => index !== "")
      .join(","),
  ].join(":");
}

function drawMessages(opened) {
  state.channelSignature = signatureOf(opened);
  const channel = state.channels.find((c) => c.id === opened.channel);
  el("channel-name").textContent = channel ? `#${channel.name}` : "channel";
  el("channel-topic").textContent = channel ? channel.topic : "";

  const list = el("messages");
  // Measured before the list is emptied, since an empty list is always "at the
  // bottom" and every redraw would then scroll.
  const atBottom =
    list.scrollHeight - list.scrollTop - list.clientHeight < 40 || list.children.length === 0;
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

    // Said in words, like `edited`. A pinned message used to be marked only by a
    // 2px inset shadow on a row with 4px of padding — present, invisible, and
    // indistinguishable from the pin having done nothing at all.
    if (message.pinned) {
      const pinned = document.createElement("span");
      pinned.className = "flag pinned-flag";
      pinned.textContent = "pinned";
      row.append(pinned);
    }

    // Appended *after* the actions below, so read the order at the end of this
    // loop rather than here.

    // Anything that is not a vote, kept visible. The record carries a free-form
    // key (spec 07 §3), so another client may write reactions this one does not
    // offer — rendering them as chips is the difference between "this client
    // has no button for that" and "that never happened".
    for (const reaction of message.reactions) {
      if (reaction.key === UP || reaction.key === DOWN) continue;
      const chip = document.createElement("button");
      chip.className = reaction.mine ? "reaction mine" : "reaction";
      chip.textContent = `${reaction.key} ${reaction.count}`;
      chip.title = reaction.mine ? "take this back" : "react";
      chip.addEventListener("click", () =>
        act(() =>
          invoke("react", {
            channel: opened.channel,
            message: message.id,
            key: reaction.key,
            remove: reaction.mine,
          }),
        ),
      );
      row.append(chip);
    }

    // Actions first, votes last, so the votes sit flush against the right edge
    // of every row. The other way round put them immediately left of the action
    // bar — whose width depends on what this member may do to *this* message —
    // so votes on your own messages, which carry edit and withdraw, sat out of
    // line with votes on everybody else's.
    row.append(actions(opened.channel, message));
    if (!message.withdrawn) row.append(votes(opened.channel, message));
    list.append(row);
  }

  // Only when they were already at the bottom. Now that a redraw happens on a
  // timer rather than only on their own action, scrolling to the end
  // unconditionally would drag a reader out of the history they scrolled up to
  // read, every two seconds.
  if (atBottom) list.scrollTop = list.scrollHeight;

  // A record this node refused is one another client may be showing. Silence
  // would make the two look like they agree.
  const refused = el("refused");
  refused.hidden = opened.refused.length === 0;
  refused.textContent = opened.refused.length
    ? `refused ${opened.refused.length} record(s): ${opened.refused.join(", ")}`
    : "";
}

/// Re-reads what replay decides — this member's standing, and the channels.
///
/// Drawn only when either actually changed, since both replay the governance log
/// and redrawing the sidebar on a timer would fight anybody using it.
async function refreshReplayed() {
  const me = await invoke("me");
  if (signatureOfMe(me) !== state.meSignature) drawMe(me);

  const rows = await invoke("sidebar");
  if (signatureOfSidebar(rows) !== state.sidebarSignature) drawSidebar(rows);
}

/// What this member may do, and is called. Not the network id, which cannot
/// change while one is open.
function signatureOfMe(me) {
  return [
    me.name,
    me.has_key,
    me.may_post,
    me.may_create_channel,
    me.may_manage_channel,
    me.may_invite,
    me.may_moderate,
    me.may_set_relays,
  ].join(":");
}


/// Re-reads the open channel on a timer, whatever the events did.
///
/// **The fourth bug in three days where a pushed event was the only path to a
/// redraw**, and the reason this is a poll rather than another attempt to get
/// the event right. `design/05` §3 already says a consumer merges rather than
/// appends, and the interface already re-reads from the projection on every
/// draw — so asking again costs a replay and can never show the wrong thing.
///
/// Cheap because it compares before it draws: replaying a channel is local, and
/// the DOM work is what actually costs, so an unchanged channel does nothing.
/// `design/05` §5's projection is what makes the replay itself cheap later.
function watchChannel() {
  if (state.channelPoll) clearInterval(state.channelPoll);
  state.channelPoll = setInterval(async () => {
    try {
      // The sidebar and the header come out of replay too, and a channel
      // defined by somebody else changes neither the open channel nor anything
      // this window did. Verified in `two_nodes.rs`: a channel created after a
      // member joins does reach them — so a founder making one that the joiner
      // never saw was this list not being redrawn, not the entry not arriving.
      await refreshReplayed();
      await drawPeople();
    } catch {
      // Same reasoning as below: a background tick reports nothing.
    }
    if (!state.current) return;
    try {
      const opened = await invoke("open_channel", { channel: state.current });
      if (signatureOf(opened) === state.channelSignature) return;
      drawMessages(opened);
    } catch {
      // A channel that cannot be opened right now — mid-restart, or not keyed
      // yet — is not something to report from a background timer. The paths a
      // person actually triggered say so where they are looking.
    }
  }, CHANNEL_REFRESH_MILLIS);
}

/// Unread counts, per channel, for this person on this machine.
///
/// Local by construction and deliberately so: what somebody has read is not a
/// fact about the network, and writing it to the log would publish a reading
/// habit to every member. `localStorage` is per-origin and per-device, which is
/// exactly the scope wanted.
///
/// Keyed by network, since one window opens several and a channel id is only
/// unique within one.
function unreadKey() {
  return `kols:unread:${state.me?.network ?? "none"}`;
}

function rememberUnread() {
  try {
    localStorage.setItem(unreadKey(), JSON.stringify(state.unread));
  } catch {
    // A window that cannot store this still counts unread for the session.
    // Losing it on restart is worth less than failing to draw anything.
  }
}

function recallUnread() {
  try {
    state.unread = JSON.parse(localStorage.getItem(unreadKey()) ?? "{}") ?? {};
  } catch {
    state.unread = {};
  }
}

async function openChannel(id) {
  // Reading it is what marks it read. Done before the render so the count is
  // gone by the time the sidebar is drawn below.
  if (state.unread[id]) {
    delete state.unread[id];
    rememberUnread();
  }
  state.current = id;
  drawSidebar(state.sidebar);

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
  drawSidebar(await invoke("sidebar"));
  // Asked on every refresh, not only on the event. A window that opened after
  // the node reported would otherwise never hear about a healed fork, and the
  // whole point of the report is that somebody hears about it.
  await drawReorg();
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

el("new-relay-identity").addEventListener("click", async () => {
  const button = el("new-relay-identity");
  button.disabled = true;
  try {
    const phrase = await invoke("new_relay_identity");
    el("relay-phrase").hidden = false;
    el("relay-phrase-text").value = phrase;
    el("relay-phrase-text").select();
  } catch (err) {
    el("relay-error").hidden = false;
    el("relay-error").className = "relay-error";
    el("relay-error").textContent = String(err);
  } finally {
    button.disabled = false;
  }
});

el("copy-phrase").addEventListener("click", async () => {
  const phrase = el("relay-phrase-text").value;
  if (!phrase) return;
  await navigator.clipboard.writeText(phrase);
  el("copy-phrase").textContent = "copied";
  setTimeout(() => (el("copy-phrase").textContent = "copy"), 1500);
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
    state.restartedAt = Date.now();
    el("relay-help").hidden = true;
    await drawRelays();
    watchRelay();
    error.hidden = false;
    error.className = "relay-error good";
    error.textContent =
      "designated, and this node is restarting onto it. Every other member " +
      "learns it by replay. The line above says what happens next — it takes " +
      "up to about 20 seconds to settle.";
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

el("reorg-dismiss").addEventListener("click", () => {
  // Dismissed by hand and never on a timer. The node does not re-report a heal
  // it has already announced, so a banner that faded would be the only notice
  // anybody got, gone.
  el("reorg").hidden = true;
});

el("new-folder").addEventListener("click", async () => {
  const name = await askFor("Name the new folder", { placeholder: "staff" });
  if (!name?.trim()) return;
  // Placed at the end: the last existing position plus a gap, so the first drag
  // that reorders anything has room to split without renumbering.
  const last = state.sidebar
    .filter((row) => row.kind === "category")
    .reduce((most, row) => Math.max(most, row.position ?? 0), 0);
  await act(async () => {
    await invoke("create_category", { name, position: last + 1024 });
    await refresh();
  });
});

el("new-channel").addEventListener("click", async () => {
  const name = await askFor("Name the new channel", { placeholder: "general" });
  if (!name?.trim()) return;
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

/// Shows one settings panel and marks its tab.
///
/// The sections are grouped by what a click costs rather than by topic
/// (`design/09` §4.2): a font is local and undoable, a network's name is a
/// governance entry every member replays and nobody can unwrite. Which side of
/// that line a panel sits on is the nav's job to say, so switching tabs never
/// changes the grouping — only which panel is open.
function showSettings(tab) {
  state.settingsTab = tab;
  for (const panel of document.querySelectorAll(".settings-panel")) {
    panel.hidden = panel.dataset.panel !== tab;
  }
  for (const button of document.querySelectorAll(".settings-tab")) {
    button.classList.toggle("current", button.dataset.tab === tab);
  }
}

/// Fills whichever panel is open with what it needs.
///
/// Per panel rather than all at once: roles are a governance replay and the
/// relay panel asks the node for its standing, and neither is worth doing for
/// a tab nobody is looking at.
async function drawSettings() {
  const me = state.me;
  if (!me) return;

  el("identity-name").value = me.name ?? "";
  el("network-name-input").value = me.network_name ?? "";
  // Gated on the capability, like every other control here. Hiding is
  // presentation only — the command is re-checked on receipt regardless.
  el("network-name-form").hidden = !me.may_set_relays;
  el("new-role").hidden = !me.may_define_group;

  if (state.settingsTab === "network") {
    await drawRelays();
    drawAdmission(me);
    await drawNetworkSettings();
  }
  if (state.settingsTab === "permissions") await drawRoles();
}

// ── what the network runs on ───────────────────────────────────────────

/// Which admission mode is in force, and whether it may be changed.
///
/// Auto-admit is *disabled rather than absent* on a member-vote network, with
/// the reason beside it. Hiding it would leave somebody looking for a setting
/// the client had silently decided not to offer; saying why is what turns Core
/// §2.6's incompatible pairing into something a person can act on.
function drawAdmission(me) {
  const box = el("admission");
  const note = el("admission-note");
  for (const input of box.querySelectorAll("input")) {
    input.checked = input.value === me.admission_mode;
    // The command is re-checked on receipt regardless — `design/09` §5. This
    // decides what is offered, never what is allowed.
    input.disabled = !me.may_set_relays || (input.value === "auto" && me.member_vote);
  }
  note.hidden = !me.member_vote;
  note.textContent = me.member_vote
    ? "This network decides admission by member vote, so it cannot also admit automatically — a quorum has to be able to decide something."
    : "";
}

/// Every chat setting, split into the ones that refuse and the ones that do not.
async function drawNetworkSettings() {
  let settings;
  try {
    settings = await invoke("settings");
  } catch (err) {
    el("setting-error").hidden = false;
    el("setting-error").textContent = String(err);
    return;
  }
  const may = Boolean(state.me?.may_set_relays);
  fillSettings(el("limit-list"), settings.filter((s) => !s.retention), may);
  fillSettings(el("retention-list"), settings.filter((s) => s.retention), may);
}

function fillSettings(list, settings, may) {
  list.replaceChildren();
  for (const setting of settings) {
    const item = document.createElement("li");
    item.className = "setting";

    const head = document.createElement("div");
    head.className = "setting-head";

    const label = document.createElement("span");
    label.className = "setting-label";
    label.textContent = setting.label;

    const value = document.createElement("span");
    value.className = "setting-value";
    value.textContent = renderSetting(setting);
    head.append(label, value);

    if (may) {
      const edit = document.createElement("button");
      edit.className = "perm-add";
      edit.textContent = "change";
      edit.addEventListener("click", () => changeSetting(setting));
      head.append(edit);
    }
    item.append(head);

    const why = document.createElement("p");
    why.className = "dim small";
    why.textContent = setting.summary;
    item.append(why);

    // A network riding the default picks up a revised one; a network that wrote
    // the same number does not. Saying which is cheap and the two are otherwise
    // indistinguishable on screen.
    const source = document.createElement("p");
    source.className = "dim small";
    source.textContent = setting.explicit
      ? `set by this network — the shipped default is ${plain(setting, setting.default)}`
      : "the shipped default, never set here";
    item.append(source);

    list.append(item);
  }
}

/// A setting's current value, said the way it actually behaves.
function renderSetting(setting) {
  if (setting.value === 0) {
    // Zero does not mean one thing across these, and the difference is the part
    // worth spelling out rather than rendering as a bare `0`.
    if (setting.zero_means.startsWith("no limit")) return "no limit";
    if (setting.zero_means.startsWith("kept forever")) return "forever";
  }
  return plain(setting, setting.value);
}

/// A number with its unit, sizes folded to KiB or MiB.
function plain(setting, value) {
  switch (setting.unit) {
    case "bytes":
      return bytes(value);
    case "per-minute":
      return `${value} a minute`;
    case "millis":
      return value % 1000 === 0 ? `${value / 1000}s` : `${value}ms`;
    case "seconds":
      return value % 3600 === 0 && value !== 0 ? `${value / 3600}h` : `${value}s`;
    case "days":
      return value === 0 ? "forever" : `${value} days`;
    default:
      return String(value);
  }
}

function bytes(value) {
  if (value >= 1024 * 1024 && value % (1024 * 1024) === 0)
    return `${value / (1024 * 1024)} MiB`;
  if (value >= 1024 && value % 1024 === 0) return `${value / 1024} KiB`;
  return `${value} bytes`;
}

/// Changes one setting, asking for a plain number in the unit it is stored in.
async function changeSetting(setting) {
  const typed = await askFor(`${setting.label} — ${unitWord(setting)}`, {
    value: String(setting.value),
    placeholder: String(setting.default),
  });
  if (typed === null) return;

  const value = Number(typed.trim());
  if (!Number.isInteger(value)) {
    await settingsAct("setting-error", () => {
      throw new Error("that needs to be a whole number");
    });
    return;
  }
  // Said before it is written rather than discovered afterwards. Zero is a
  // legitimate value for every one of these and means something different for
  // each, so the confirmation names the actual consequence — native, because
  // this asks whether to sign rather than what to write (`design/09` §5.1).
  if (
    value === 0 &&
    !confirm(`Set ${setting.label} to zero?\n\nThat means ${setting.zero_means}.`)
  )
    return;

  await settingsAct("setting-error", async () => {
    await invoke("set_chat_setting", { setting: setting.id, value });
    await drawNetworkSettings();
  });
}

/// What unit a setting's box wants, so nobody types "8 KiB" into a byte count.
function unitWord(setting) {
  switch (setting.unit) {
    case "bytes":
      return "in bytes";
    case "per-minute":
      return "per minute";
    case "millis":
      return "in milliseconds";
    case "seconds":
      return "in seconds";
    case "days":
      return "in days, or 0 for forever";
    default:
      return "a whole number";
  }
}

for (const input of document.querySelectorAll('#admission input')) {
  input.addEventListener("change", async () => {
    await settingsAct("admission-error", async () => {
      await invoke("set_admission_mode", { mode: input.value });
      drawMe(await invoke("me"));
      drawAdmission(state.me);
    });
  });
}

for (const button of document.querySelectorAll(".settings-tab")) {
  button.addEventListener("click", async () => {
    showSettings(button.dataset.tab);
    await drawSettings();
  });
}

el("open-settings").addEventListener("click", async () => {
  el("settings").hidden = false;
  showSettings(state.settingsTab ?? "network");
  // Asked for on open rather than kept warm: the panel is closed almost always,
  // and a relay's standing is only interesting when somebody is looking at it.
  await drawSettings();
});

el("close-settings").addEventListener("click", () => {
  el("settings").hidden = true;
});

// Escape closes it, since a panel over everything with one way out is a trap
// the first time somebody opens it by accident.
document.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && !el("settings").hidden) el("settings").hidden = true;
});

el("identity-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const name = el("identity-name").value.trim();
  if (!name) return;
  await settingsAct("identity-error", async () => {
    await invoke("set_name", { name });
    drawMe(await invoke("me"));
    await drawSettings();
  });
});

el("network-name-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  await settingsAct("network-name-error", async () => {
    await invoke("set_network_name", { name: el("network-name-input").value.trim() });
    drawMe(await invoke("me"));
    await drawSettings();
  });
});

// ── permissions ────────────────────────────────────────────────────────
//
// Role-first, because permissions are held by roles and never by people
// (`design/02` §1): there is no per-user grant anywhere in this protocol, so a
// person-first surface would be a lie about what the log can express. Giving one
// person access means a role containing only them, which `giveAccessTo` does
// rather than making somebody discover it.

/// Every role, with the one being looked at expanded beside the list.
async function drawRoles() {
  let roles;
  try {
    roles = await invoke("roles");
  } catch (err) {
    el("perms-error").hidden = false;
    el("perms-error").textContent = String(err);
    return;
  }

  const list = el("role-list");
  list.replaceChildren();
  for (const role of roles) {
    const item = document.createElement("li");
    const button = document.createElement("button");
    button.className = role.id === state.role ? "current" : "";
    button.addEventListener("click", async () => {
      state.role = role.id;
      await drawRoles();
    });

    const name = document.createElement("span");
    name.className = "role-name";
    name.textContent = role.id;
    button.append(name);

    // What a role actually confers, at a glance. A count of members alone would
    // make an empty powerful role and an empty powerless one look identical.
    const note = document.createElement("span");
    note.className = "role-note";
    note.textContent = role.unrestricted
      ? "everything"
      : `${role.grants.length + role.protocol_grants.length} · ${role.members.length} in`;
    button.append(note);

    item.append(button);
    list.append(item);
  }

  const chosen = roles.find((role) => role.id === state.role);
  drawRoleDetail(chosen ?? null);
}

/// One role: who is in it, and what it can do.
function drawRoleDetail(role) {
  const box = el("perms-detail");
  box.replaceChildren();
  if (!role) {
    const hint = document.createElement("p");
    hint.className = "dim";
    hint.textContent = "Choose a role.";
    box.append(hint);
    return;
  }

  const title = document.createElement("h4");
  title.textContent = role.id;
  box.append(title);

  if (role.implicit) {
    const note = document.createElement("p");
    note.className = "dim small";
    note.textContent = role.everyone
      ? "Every member is in this one on arrival. It may never hold a governance-tier permission — otherwise being let in would itself confer it."
      : "Created with the network, holding every capability there is.";
    box.append(note);
  }

  box.append(memberSection(role));
  box.append(grantSection(role));
}

/// Who holds this role.
function memberSection(role) {
  const section = document.createElement("section");
  section.className = "perm-block";

  // Its own header rule rather than the sidebar's `channels-head`: that one
  // carries the rail's spacing and uppercase treatment, and borrowing it here
  // dragged a column of sidebar styling into a settings sheet.
  const head = document.createElement("div");
  head.className = "perm-head-row";
  const heading = document.createElement("h5");
  heading.textContent = `members (${role.members.length})`;
  head.append(heading);

  if (role.may_assign && !role.everyone) {
    const add = document.createElement("button");
    add.className = "perm-add";
    add.textContent = "+";
    add.title = "add somebody";
    add.addEventListener("click", () => addToRole(role));
    head.append(add);
  }
  section.append(head);

  const list = document.createElement("ul");
  list.className = "role-members";
  for (const member of role.members) {
    const item = document.createElement("li");

    const who = document.createElement("span");
    who.className = "role-member-name";
    who.textContent = member.name ?? member.short;
    // Spec 07 §8: a name is never sufficient on its own. Uniqueness is decided
    // on a key that does not fold lookalikes, so the identity rides alongside.
    const id = document.createElement("span");
    id.className = "mono small";
    id.textContent = member.short;
    item.append(who, id);

    if (role.may_assign && !role.everyone) {
      const drop = document.createElement("button");
      drop.className = "drop";
      drop.textContent = "×";
      drop.title = `take ${member.name ?? member.short} out of ${role.id}`;
      drop.addEventListener("click", async () => {
        // Native, because this asks *whether* rather than *what* (§5.1), and
        // because taking yourself out of a powerful role is the one action here
        // that can leave a network nobody can govern. `design/02` §5 refuses a
        // hierarchy that would prevent it — there is no structural protection
        // for a founder, only who holds the capability — so the client warns and
        // does not block. Blocking would be inventing the hierarchy the protocol
        // declines to have.
        if (
          member.you &&
          !confirm(
            `Take yourself out of ${role.id}?\n\nYou lose whatever it holds. ` +
              `If you are the last member of a role that holds governance power, ` +
              `nobody will be able to grant it back.`,
          )
        )
          return;
        await settingsAct("perms-error", async () => {
          await invoke("set_role_member", {
            role: role.id,
            identity: member.identity,
            member: false,
          });
          await drawRoles();
        });
      });
      item.append(drop);
    }
    list.append(item);
  }
  if (role.members.length === 0) {
    const empty = document.createElement("li");
    empty.className = "empty";
    empty.textContent = role.everyone ? "every member" : "nobody yet";
    list.append(empty);
  }
  section.append(list);

  if (role.everyone) {
    const note = document.createElement("p");
    note.className = "dim small";
    note.textContent =
      "Membership here is admission to the network, and is managed from the waiting room rather than as a role.";
    section.append(note);
  }
  return section;
}

/// What this role can do, and where.
function grantSection(role) {
  const section = document.createElement("section");
  section.className = "perm-block";

  const heading = document.createElement("h5");
  heading.textContent = "can do";
  section.append(heading);

  if (role.unrestricted) {
    const note = document.createElement("p");
    note.className = "dim small";
    note.textContent =
      "Everything, including permissions defined later. There is no set to take one out of — narrowing this means replacing it with an explicit list, which is a larger act than a checkbox and is not offered here.";
    section.append(note);
    return section;
  }

  const list = document.createElement("ul");
  list.className = "grant-list";
  for (const grant of role.grants) {
    const item = document.createElement("li");
    item.className = grant.governance ? "grant governance" : "grant";

    const verb = document.createElement("span");
    verb.className = "grant-verb";
    verb.textContent = grant.verb;

    const where = document.createElement("span");
    where.className = "grant-scope";
    where.textContent = grant.scope_label;

    item.append(verb, where);

    if (state.me?.may_define_group) {
      const drop = document.createElement("button");
      drop.className = "drop";
      drop.textContent = "×";
      drop.title = `withdraw ${grant.verb} at ${grant.scope_label}`;
      drop.addEventListener("click", async () => {
        await settingsAct("perms-error", async () => {
          await invoke("set_permission", {
            role: role.id,
            verb: grant.verb,
            scope: grant.scope,
            scopeId: grant.scope_id,
            grant: false,
          });
          await drawRoles();
        });
      });
      item.append(drop);
    }
    list.append(item);
  }

  // Shown, never edited. These are the network's own governance rather than this
  // application's vocabulary, and a grid built for chat verbs would present them
  // as the same kind of thing. A role whose powers were half displayed would
  // read as weaker than it is.
  for (const name of role.protocol_grants) {
    const item = document.createElement("li");
    item.className = "grant protocol";
    const verb = document.createElement("span");
    verb.className = "grant-verb";
    verb.textContent = name;
    const where = document.createElement("span");
    where.className = "grant-scope";
    where.textContent = "the network's own";
    item.append(verb, where);
    list.append(item);
  }

  if (list.childElementCount === 0) {
    const empty = document.createElement("li");
    empty.className = "empty";
    empty.textContent = "nothing yet";
    list.append(empty);
  }
  section.append(list);

  if (state.me?.may_define_group) {
    const add = document.createElement("button");
    add.className = "grant-add";
    add.textContent = "+ grant a permission";
    add.addEventListener("click", () => grantTo(role));
    section.append(add);
  }
  return section;
}

/// Grants one verb at one scope, chosen from what this network actually has.
///
/// A picker rather than a text field, deliberately: an unregistered capability
/// name is refused at replay, so a typo would produce a grant that resolves for
/// nobody and reports nothing — the failure mode a free-text field invites.
async function grantTo(role) {
  const [verbs, scopes] = await Promise.all([invoke("verbs"), invoke("scopes")]);

  const allowed = verbs.filter((verb) => !(role.everyone && verb.governance));
  const verb = await pickOne(
    `What may ${role.id} do?`,
    allowed.map((v) => [
      v.name,
      v.governance ? `${v.summary} — governance-tier` : v.summary,
    ]),
  );
  if (!verb) return;

  const scope = await pickOne(
    `Where may ${role.id} ${verb}?`,
    // Categories first, because `design/02` §4 makes the category the scope a
    // grant is expected to bind at and the channel the override.
    scopes.map((s) => [`${s.kind}:${s.id}`, s.label]),
  );
  if (!scope) return;
  const [kind, id] = splitScope(scope);

  await settingsAct("perms-error", async () => {
    await invoke("set_permission", {
      role: role.id,
      verb,
      scope: kind,
      scopeId: id,
      grant: true,
    });
    await drawRoles();
  });
}

/// Splits a `kind:id` scope key without eating a hex id that contains no colon.
function splitScope(key) {
  const at = key.indexOf(":");
  return [key.slice(0, at), key.slice(at + 1)];
}

/// Puts somebody in a role, chosen from the people this network has.
async function addToRole(role) {
  const people = await invoke("people");
  const held = new Set(role.members.map((member) => member.identity));
  const candidates = people.filter((person) => !held.has(person.identity));
  if (candidates.length === 0) {
    await settingsAct("perms-error", () => {
      throw new Error("everybody here already holds that role");
    });
    return;
  }

  const who = await pickOne(
    `Who joins ${role.id}?`,
    candidates.map((person) => [
      person.identity,
      `${person.name ?? person.short} · ${person.short}`,
    ]),
  );
  if (!who) return;

  await settingsAct("perms-error", async () => {
    await invoke("set_role_member", { role: role.id, identity: who, member: true });
    await drawRoles();
  });
}

// ── asking ─────────────────────────────────────────────────────────────
//
// **The line, stated once because both sides of it look like a dialog.**
// `design/09` §6.5 requires anything that asks a member to *authorise*
// something to sit outside the document a theme can reach — a theme may hide,
// move or cover any element, so a confirmation it can conceal is not one. That
// applies to asking *whether*, and the destructive confirmations keep using
// `window.confirm`, which is browser chrome and outside the DOM by
// construction.
//
// Asking *what* is a different act. Choosing a verb, picking a scope, typing a
// name — none of it approves anything, and a theme that restyled it can at
// worst make its own client awkward. So those live here, in the document,
// where they can look like the rest of the application instead of like the
// operating system's idea of a text field.

/// The shell both askers share: a veil, a sheet, and a cancel that resolves null.
function dialog(question, fill) {
  return new Promise((resolve) => {
    document.querySelector(".chooser")?.remove();

    let settled = false;
    const close = (value) => {
      if (settled) return;
      settled = true;
      veil.remove();
      document.removeEventListener("keydown", onKey);
      resolve(value);
    };
    const onKey = (event) => {
      if (event.key === "Escape") close(null);
    };

    const veil = document.createElement("div");
    veil.className = "chooser";
    veil.dataset.kols = "chooser";

    const sheet = document.createElement("div");
    sheet.className = "chooser-sheet";

    const title = document.createElement("h4");
    title.textContent = question;
    sheet.append(title);

    fill(sheet, close);

    const cancel = document.createElement("button");
    cancel.className = "chooser-cancel";
    cancel.textContent = "cancel";
    cancel.addEventListener("click", () => close(null));
    sheet.append(cancel);

    veil.append(sheet);
    veil.addEventListener("click", (event) => {
      if (event.target === veil) close(null);
    });
    document.addEventListener("keydown", onKey);
    document.body.append(veil);
  });
}

/// One of a fixed set, resolving to the chosen value or null.
function pickOne(question, options) {
  return dialog(question, (sheet, close) => {
    const list = document.createElement("ul");
    for (const [value, label] of options) {
      const item = document.createElement("li");
      const button = document.createElement("button");
      button.textContent = label;
      button.addEventListener("click", () => close(value));
      item.append(button);
      list.append(item);
    }
    sheet.append(list);
  });
}

/// A line of text, resolving to what was typed or null.
///
/// Distinguishes cancelling from clearing, like the `prompt` it replaces: an
/// empty answer is a value the caller may accept — unnaming a network is a real
/// act — and null is the member deciding not to.
function askFor(question, { value = "", placeholder = "" } = {}) {
  return dialog(question, (sheet, close) => {
    const form = document.createElement("form");
    form.className = "chooser-form";

    const input = document.createElement("input");
    input.type = "text";
    input.autocomplete = "off";
    input.value = value;
    input.placeholder = placeholder;
    input.setAttribute("aria-label", question);

    const submit = document.createElement("button");
    submit.type = "submit";
    submit.textContent = "ok";

    form.addEventListener("submit", (event) => {
      event.preventDefault();
      close(input.value);
    });
    form.append(input, submit);
    sheet.append(form);
    // After append, or there is nothing on screen to focus.
    setTimeout(() => {
      input.focus();
      input.select();
    }, 0);
  });
}

el("new-role").addEventListener("click", async () => {
  const name = await askFor("Name the new role", { placeholder: "Moderators" });
  if (!name?.trim()) return;
  await settingsAct("perms-error", async () => {
    await invoke("create_role", { name: name.trim() });
    state.role = name.trim();
    await drawRoles();
  });
});

/// Runs a settings action, reporting a refusal into that panel's own line.
///
/// Per panel rather than one shared banner: a refusal belongs beside the control
/// that earned it, and the channel screen's `refused` line is behind this sheet
/// where nobody would see it.
async function settingsAct(errorId, run) {
  const line = el(errorId);
  try {
    await run();
    line.hidden = true;
  } catch (err) {
    line.hidden = false;
    line.textContent = String(err);
  }
}

/// What the node learns, while it runs.
///
/// Every one of these re-reads rather than patching what is on screen. That is
/// `design/05` §3's third property in the smallest form it takes: a record that
/// arrived over gossip is also inside the segment that follows it, so a consumer
/// that appended what it was handed would show every message twice. Re-reading
/// makes a duplicate a non-event, with no bookkeeping to get wrong.
async function watch() {
  // Any records at all, not only records naming the open channel.
  //
  // The comparison that used to be here could skip a redraw and could never
  // cause one, which makes it a micro-optimisation whose only possible effect is
  // the bug it produced: a message from the other side sat unrendered until the
  // reader posted, at which point the composer's own re-read revealed it. The
  // records were in the store the whole time.
  await listen("kols://records", async (event) => {
    const [channel, messages] = event.payload;
    // Unread is driven by arrival rather than by scanning, which is what makes
    // it free: the node reports what it learned, and a channel nobody is
    // looking at gains a count. It survives the app being closed for the same
    // reason — the node was not running either, so it learns the backlog on the
    // next start and reports it then.
    if (messages && channel !== state.current) {
      state.unread[channel] = (state.unread[channel] ?? 0) + 1;
      rememberUnread();
      drawSidebar(state.sidebar);
    }
    if (state.current) await openChannel(state.current);
  });

  // Channels, permissions and names all come out of replay, so anything that
  // moved the log may have made the sidebar and the header stale.
  await listen("kols://governance", async () => {
    drawMe(await invoke("me"));
    drawSidebar(await invoke("sidebar"));
    // Relays are policy, so an entry that moved the log may have changed them —
    // which is how a member learns of a relay designated after they joined. The
    // only place that passes `act`: a change learned from the log is the case
    // where this node is running without having dialled what the network now
    // names.
    await drawRelays({ act: true });
    if (state.current) await openChannel(state.current);
  });

  // Somebody presented an invite. The node has already written the waiting room
  // down by the time this arrives, so this only has to re-read it.
  //
  // Its absence was the whole of one bug: the event was emitted and nothing
  // listened, so a founder watching the door saw nobody at it while the joiner
  // waited to be let in. The same shape as the relay panel missing its report,
  // and the reason the doorway now polls as well.
  await listen("kols://joins", async () => {
    drawMe(await invoke("me"));
  });

  await listen("kols://keys", async () => {
    drawMe(await invoke("me"));
  });

  // The node's standing with the relay, reported once at startup — and
  // reported on success, which is the half a window never used to get.
  await listen("kols://relay", async () => {
    // The payload is not read: the node holds this answer and `relays` returns
    // it, so the event's only job is to say "ask again now" rather than to be
    // the answer itself. That is what makes a missed one harmless.
    await drawRelays();
  });

  await listen("kols://reorg", async () => {
    await drawReorg();
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
  // After `drawMe`, which is what puts the network id in `state.me` — the key
  // these are stored under.
  recallUnread();
  watchRelay();
  watchChannel();
  const rows = await invoke("sidebar");
  drawSidebar(rows);
  const channels = state.channels;
  if (channels.length > 0) await openChannel(channels[0].id);
}

watch();
start();
