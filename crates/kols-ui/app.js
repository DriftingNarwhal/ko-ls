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

function channelMenu(event, channel, category) {
  const folders = state.sidebar.filter((row) => row.kind === "category");
  const entries = [
    [
      "rename",
      async () => {
        const name = prompt("channel name", channel.name);
        if (!name) return;
        await act(async () => {
          await invoke("rename_channel", { channel: channel.id, name });
          await refresh();
        });
      },
    ],
    [
      "set topic",
      async () => {
        const topic = prompt("channel topic", channel.topic ?? "");
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
  popMenu(event, [
    ["move up", async () => nudgeFolder(row, -1)],
    ["move down", async () => nudgeFolder(row, 1)],
    [
      "rename",
      async () => {
        const name = prompt("folder name", row.name);
        if (!name) return;
        await act(async () => {
          await invoke("rename_category", { category: row.id, name });
          await refresh();
        });
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
    button("edit", "revise this", () => {
      const body = prompt("revise this message", message.body);
      // Distinguished deliberately: cancelling is not the same as clearing, and
      // an empty edit is refused by the gate rather than silently dropped here.
      if (body === null) return Promise.resolve();
      return invoke("edit_message", { channel, message: message.id, body });
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
  const name = prompt("folder name");
  if (!name) return;
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

el("open-settings").addEventListener("click", async () => {
  el("settings").hidden = false;
  // Asked for on open rather than kept warm: the panel is closed almost always,
  // and a relay's standing is only interesting when somebody is looking at it.
  await drawRelays();
});

el("close-settings").addEventListener("click", () => {
  el("settings").hidden = true;
});

// Escape closes it, since a panel over everything with one way out is a trap
// the first time somebody opens it by accident.
document.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && !el("settings").hidden) el("settings").hidden = true;
});

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
