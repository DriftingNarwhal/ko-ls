// The workspace window — `design/09` §1.1–§1.2, D40.
//
// Everything this installation belongs to, listed in one narrow window that
// sits beside a network rather than in front of it. Clicking one draws it in a
// window of its own, so changing networks is a click rather than backing out of
// the one you are in.
//
// It holds no keys, no sockets and no files: every line here either draws
// something or calls `invoke`, which crosses `kols-api`.
//
// # What is deliberately not here
//
// Anything that writes a governance entry (`09` §1.14). A network's name, its
// relays, its admission mode, its limits and its roles are replayed by every
// member forever, and none of them becomes cheaper by being reachable from a
// list — they stay in that network's own window, behind the heading that says
// what a click costs.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const el = (id) => document.getElementById(id);

/// Which screen is showing. The lock comes before everything, as it does in the
/// network window, and for the same reason (`02` §6.3).
function show(view) {
  el("lock").hidden = view !== "lock";
  el("workspace").hidden = view !== "workspace";
  el("settings").hidden = view !== "settings";
}

function fail(err) {
  const line = el("workspace-error");
  line.hidden = false;
  // Cleared rather than assumed absent: `said` dims this same line to report a
  // departure that worked, and a refusal arriving after one would otherwise be
  // drawn as though it had also gone well.
  line.classList.remove("told");
  line.textContent = String(err && err.message ? err.message : err);
}

/// Reports a departure at the strength it holds — `02` §6.5.
///
/// What went out is *who could have heard*, never who did: gossip acknowledges
/// nothing. Zero is said plainly rather than softened, because it is
/// unrecoverable — the seed that signed the entry is gone, so it cannot be sent
/// again.
function said(outcome) {
  const line = el("workspace-error");
  line.hidden = false;
  if (!outcome.announced || outcome.reached === 0) {
    line.classList.remove("told");
    line.textContent = outcome.reason || "Removed here. The network was not told.";
    return;
  }
  const who =
    outcome.reached === 1 ? "1 connected member" : `${outcome.reached} connected members`;
  line.classList.add("told");
  line.textContent =
    `Left, and the departure went out to ${who}. Anybody offline learns of it from them.`;
}

/// Draws the list.
///
/// One row per network, and for now that is all: the row contents `09` §1.3
/// specifies — unread, whether this node holds a connection, the tier and when
/// a cold one last looked — need shell state that does not exist yet, and a row
/// that displayed a zero it had not earned would be making exactly the claim
/// that section forbids.
async function draw() {
  await drawNetworks();
  await drawConversations();
  show("workspace");
}

/// Redrawn from what the shell answers rather than patched in place, and
/// **skipped entirely when the answer has not changed** — a five-second poll
/// that rebuilt these rows would take the row out from under a pointer already
/// on its way to it, which is the same reason the network window keeps
/// signatures.
let lastNetworks = "";

async function drawNetworks() {
  const networks = await invoke("networks");
  const fingerprint = JSON.stringify(networks);
  if (fingerprint === lastNetworks) return;
  lastNetworks = fingerprint;

  const list = el("networks");
  list.replaceChildren();

  for (const network of networks) {
    const item = document.createElement("li");
    item.className = "workspace-row";
    if (network.open) item.classList.add("open");

    const button = document.createElement("button");
    button.className = "workspace-open";
    button.textContent = network.label || network.id.slice(0, 12);
    button.addEventListener("click", () => void open(network.id));

    const note = document.createElement("span");
    note.className = "workspace-note";
    // Not "broken": a network you have joined and not yet been keyed into is a
    // normal place to be, and saying so beats an empty channel list.
    note.textContent = network.keyed ? "" : "not keyed in yet";

    item.append(button, note, forgetButton(network));
    list.append(item);
  }

  el("networks-empty").hidden = networks.length > 0;
}

/// The conversations group — `design/09` §1.4, §1.8.
///
/// Three states in one list, because they are one thing to the member:
/// somebody who asked, somebody who was asked, and somebody being talked to.
let lastConversations = "";

async function drawConversations() {
  let found = [];
  try {
    found = await invoke("conversations");
  } catch {
    // A list that cannot be read is not worth a banner over the networks that
    // can. It is asked for again on the next draw.
    return;
  }
  const fingerprint = JSON.stringify(found);
  if (fingerprint === lastConversations) return;
  lastConversations = fingerprint;

  const list = el("conversations");
  list.replaceChildren();

  for (const it of found) {
    const item = document.createElement("li");
    item.className = "workspace-row";

    const open = document.createElement("button");
    open.className = "workspace-open";
    // **A name with enough identity beside it to tell two lookalikes apart** —
    // spec 07 §8, and this is the place it matters most: a contact list is
    // where somebody decides who they are talking to, and the uniqueness key
    // deliberately does not fold confusables (§3.9.1).
    open.textContent = it.label || (it.who ? it.who.slice(0, 8) : "a conversation");
    if (it.who) {
      const id = document.createElement("span");
      id.className = "workspace-id";
      id.textContent = it.who.slice(0, 8);
      open.append(id);
    }

    if (it.state === "joined") {
      open.addEventListener("click", () => void openConversation(it));
    } else {
      open.disabled = true;
    }

    item.append(open, conversationNote(it));
    if (it.state === "asked") item.append(...answerButtons(it));
    list.append(item);
  }
  el("conversations-empty").hidden = found.length > 0;
}

/// What a row says about where a conversation has got to.
function conversationNote(it) {
  const note = document.createElement("span");
  note.className = "workspace-note";
  if (it.state === "asked") {
    note.textContent = "wants to talk to you";
  } else if (it.state === "offered") {
    // **Never *waiting for an answer*.** Nothing is sent when somebody
    // declines (spec 07 §6.2), so a sender cannot tell a decline from somebody
    // who has not looked — and a row that claimed to know would be inventing
    // the difference (`09` §1.8).
    note.textContent = "asked — they will see it when you are both online";
  } else {
    note.textContent = "";
  }
  return note;
}

/// Accept and decline, which are a person's act and not the carrier's.
function answerButtons(it) {
  const accept = document.createElement("button");
  accept.className = "forget accept";
  accept.textContent = "accept";
  accept.addEventListener("click", async (event) => {
    event.stopPropagation();
    try {
      const network = await invoke("accept_conversation", {
        network: it.shared,
        from: it.who,
      });
      await draw();
      await openConversation({ ...it, network, state: "joined" });
    } catch (err) {
      fail(err);
    }
  });

  const decline = document.createElement("button");
  decline.className = "forget";
  decline.textContent = "decline";
  decline.addEventListener("click", async (event) => {
    event.stopPropagation();
    // **Native, and it says the thing neither side can see.** The sender is
    // told nothing — the carrier acknowledged delivery and there is no
    // application-level answer, deliberately, since a refusal that
    // distinguished itself would turn every decline into a disclosure
    // (spec 07 §6.2). Somebody who believed a decline was both private and
    // visible would have it exactly backwards.
    const who = it.label || it.who.slice(0, 8);
    if (
      !confirm(
        `Decline ${who}?\n\nThey are not told. From their side this looks the ` +
          `same as you not having looked yet, and they can ask again.`,
      )
    ) {
      return;
    }
    try {
      await invoke("decline_conversation", { network: it.shared, from: it.who });
      await draw();
    } catch (err) {
      fail(err);
    }
  });
  return [accept, decline];
}

/// Opens a conversation in a window of its own — `09` §1.6.
async function openConversation(it) {
  try {
    await invoke("open_conversation", { network: it.network, label: it.label ?? "" });
  } catch (err) {
    fail(err);
  }
}

/// **Forget destroys, and announces first where it can** — `02` §6.5.
///
/// Core §2.5.1 gave a member an entry they may write for themselves, so the
/// departure is signed and handed to the running node before the store goes.
/// The order is fixed, because that entry is signed by the seed this deletes.
///
/// Only the *open* network has a node to announce through, which is what decides
/// which warning is shown. Telling somebody the network will be informed when no
/// node is running would be the exact lie §6.5 is written against.
function forgetButton(network) {
  const forget = document.createElement("button");
  forget.className = "forget";
  forget.textContent = network.open ? "leave" : "forget";
  forget.title = network.open
    ? "tell this network you are leaving, then remove this installation's copy"
    : "remove this installation's copy — with no node running, the network is not told";

  forget.addEventListener("click", async (event) => {
    event.stopPropagation();
    // Native, because this asks whether to destroy something rather than what to
    // call it (`09` §5.1) — and because the seed is the one thing here with no
    // recovery path.
    const gone =
      "\n\nThis deletes the seed, which is your identity here. You cannot come " +
      "back as the same member — a later join would arrive as a stranger, and the " +
      "log would still name the member you were.";
    const told = network.open
      ? "\n\nThis network is open, so it is told: your departure is written and " +
        "published before anything is deleted. Members who are connected right now " +
        "hear it; anybody offline learns of it from them."
      : "\n\nThis network is not open, so **it is not told** — no node is running to " +
        "publish anything, and to every other member you stay a member who is simply " +
        "never connected. Open it first if you would rather it knew.";
    const loss = network.keyed
      ? gone + told
      : "\n\nYou were never keyed into this one, so there is nothing to lose but the " +
        "attempt.";
    const name = network.label || network.id.slice(0, 12);
    if (!confirm(`${network.open ? "Leave" : "Forget"} ${name}?${loss}`)) return;
    try {
      const outcome = await invoke("forget_network", { network: network.id });
      await draw();
      said(outcome);
    } catch (err) {
      fail(err);
    }
  });
  return forget;
}

/// Opens a network, which draws it in the network window.
///
/// The shell creates or raises that window and sets its title (D36) — this asks
/// for a network and never for a window, because which window a network belongs
/// in is not the interface's decision to make and a title the document composed
/// would be one the document could compose wrongly.
async function open(id) {
  try {
    await invoke("open_network", { network: id });
    await draw();
  } catch (err) {
    fail(err);
  }
}

// --- Joining and creating, as sheets ---------------------------------------
//
// They had permanent space on a page and should not have: both are done rarely,
// deliberately, and then left. Asking *what* lives in the document and is
// themeable; only asking *whether* has to be native (`09` §5.1).

/// Shortens a multiaddr for a warning that has to stay readable.
function short(address) {
  return address.length > 42 ? `${address.slice(0, 28)}…${address.slice(-6)}` : address;
}

/// Warns when a relay is already carrying another of this member's networks — D29.
///
/// **The client is the only party that can see this at all.** The relay cannot
/// (Core §5.5: it replays no log and holds no capabilities) and the other
/// network's members certainly cannot, so the notice is what this window owes
/// rather than something it happens to offer.
///
/// **It warns and never refuses** (`09` §3). A refusal would be unenforceable
/// — nothing stops a founder naming one address in two networks — so it would
/// stop the honest case and not the determined one, and it would block a member
/// legitimately relaying on their own LAN for two of their own networks.
///
/// **Creating a network with a relay is a designation too, and it is the first
/// one most people ever make.** A warning that covered only the relay panel in
/// a network's own window would miss the case it is most likely to be needed
/// for, which is exactly why this lives here as well.
async function agreedToShareRelay(relays) {
  if (!relays.trim()) return true;
  let shared;
  try {
    shared = await invoke("shared_relays_for_new_network", { relays });
  } catch {
    // A check that could not run must not stop a designation. This is the one
    // thing the client can see that nobody else can, not a gate — and failing
    // closed here would make an unrelated fault look like a refusal.
    return true;
  }
  if (shared.length === 0) return true;
  const named = shared
    .map((it) => `${short(it.relay)} — also used by ${it.label || it.id.slice(0, 12)}`)
    .join("\n");
  return confirm(
    `Another of your networks already uses this relay.\n\n${named}\n\n` +
      "Members of both may become able to discover each other, which is the " +
      "thing separate networks exist to prevent. Use it anyway?",
  );
}

function sheetError(id, err) {
  const line = el(id);
  line.hidden = false;
  line.textContent = String(err && err.message ? err.message : err);
}

el("join-open").addEventListener("click", () => {
  el("join-error").hidden = true;
  el("invite").value = "";
  el("join-sheet").showModal();
  el("invite").focus();
});

el("create-open").addEventListener("click", () => {
  el("create-error").hidden = true;
  el("new-name").value = "";
  el("new-relay").value = "";
  el("create-sheet").showModal();
  el("new-name").focus();
});

/// A join has three landings, and only one of them is a failure — O21.
///
/// Admitted and waiting are both successes (Core §2.4): under explicit intake an
/// invite buys a connection and an identity and nothing else until a member
/// admits you, and reporting that as broken describes a network working exactly
/// as configured.
///
/// The third is the one that used to be shown as a refusal and must never be
/// again. *The request reached the issuer and nothing came back* is not a no —
/// under auto-admit a network answers by writing a governance entry, so the
/// entry can exist while the reply reporting it does not. An invite is
/// use-limited, so telling somebody this failed is how they spend it on a retry
/// and lock themselves out of a network that already holds them.
// Cancel closes and does nothing else. It is `type="button"` so that Enter
// reaches the primary action rather than this one — see the markup.
el("join-cancel").addEventListener("click", () => el("join-sheet").close());
el("create-cancel").addEventListener("click", () => el("create-sheet").close());

el("joiner").addEventListener("submit", async (event) => {
  // A sheet whose form submits closes itself, so the work happens first and the
  // sheet stays open on a refusal — a dialog that vanished taking the reason
  // with it would make every failure look like nothing having happened.
  event.preventDefault();
  const invite = el("invite").value.trim();
  if (!invite) return sheetError("join-error", "paste the invite you were given");

  const go = el("join-go");
  go.disabled = true;
  go.textContent = "joining…";
  try {
    const landed = await invoke("join_network", { invite });
    el("join-sheet").close();
    await draw();

    if (landed.admitted) return;

    if (!landed.answered) {
      fail(
        `No answer yet — which is not the same as a refusal, so do not redeem ` +
          `the invite again. The network may already have admitted you; this ` +
          `node is syncing now and will say so if it did. Your identity here ` +
          `is:\n\n  ${landed.identity}`,
      );
      return;
    }

    fail(
      `You are in. This network screens its members, so you are waiting to be ` +
        `admitted — ask a member to run:\n\n  kols admit ${landed.identity}`,
    );
  } catch (err) {
    sheetError("join-error", err);
  } finally {
    go.disabled = false;
    go.textContent = "join";
  }
});

el("maker").addEventListener("submit", async (event) => {
  event.preventDefault();
  const name = el("new-name").value.trim();
  if (!name) return sheetError("create-error", "give the network a name");
  const relay = el("new-relay").value.trim();
  // D29, and the more common of its two designations — see `agreedToShareRelay`.
  if (!(await agreedToShareRelay(relay))) return;
  try {
    const network = await invoke("create_network", { name, relay });
    el("create-sheet").close();
    await draw();
    if (network && network.id) await open(network.id);
  } catch (err) {
    sheetError("create-error", err);
  }
});


// --- Closing is not quitting, said once ------------------------------------
//
// `design/09` §1.11. The shell prevents the first close and emits this instead
// of putting the window away, because a notice nobody can see is not one. Every
// close after it hides silently — the shell remembers, per installation rather
// than per launch, so this is not a thing somebody meets every morning.

listen("kols://still-running", () => {
  el("still-running").showModal();
});


// --- Keeping up with what changed while this window sat there ---------------
//
// **This window was drawn once and then went stale, which is worth naming
// because it looked like working software.** A network founded here reported
// *not keyed in yet* for as long as the window stayed open: the epoch key is
// written a beat after the node starts, the row had already been drawn, and
// nothing asked again. A request arriving from somebody was worse — §1.8 says a
// verified request appears in this list, and it appeared on the next draw,
// which might be tomorrow.
//
// Two mechanisms, because there are two kinds of change. An event is prompt and
// covers what the node announces; the poll covers what nothing announces, which
// is most of `09` §1.3's row contents — a key written at founding, a node that
// stopped, a network forgotten in another window.

/// A request arrived, or a conversation's state moved.
///
/// **Re-read rather than rendered from the payload**, like everything else in
/// this client: the request is on disk, and a consumer that drew what it was
/// handed would be appending where it should be merging.
listen("kols://conversations", () => {
  void drawConversations();
});

// Being keyed into a network changes what its row says, and a join being
// answered changes whether there is a row at all.
listen("kols://keys", () => void drawNetworks());
listen("kols://joins", () => void drawNetworks());

/// How often to ask again. Slower than the network window's two seconds,
/// because this window's rows change on the scale of somebody joining a network
/// rather than somebody typing.
const TICK_MILLIS = 5000;

setInterval(() => {
  // Only while the list is what is showing. Polling behind the lock screen
  // would be asking questions about an installation somebody has closed, and
  // polling behind settings redraws what nobody is looking at.
  if (el("workspace").hidden) return;
  void drawNetworks();
  void drawConversations();
}, TICK_MILLIS);

el("still-hide").addEventListener("click", () => {
  el("still-running").close();
  void invoke("hide_workspace");
});

// Offered here because this is the moment somebody learns the difference, and
// it is the one act that releases what this machine is holding for others.
el("still-quit").addEventListener("click", () => {
  el("still-running").close();
  void invoke("quit_app");
});

// --- Settings: what belongs to this installation ---------------------------
//
// `design/09` §1.13: §4.2's groups sort by what a click costs and now also by
// window, on the same line. *mine* — appearance, this device, the account, the
// copy that survives the disk, and the installation-wide ceiling — is about no
// network in particular, and at a first run there is no network window for it
// to live in at all.

/// Whether the export was just offered, so the nudge shows once and not forever.
let justProtected = false;

/// Which panel is showing.
function showSettings(tab) {
  for (const panel of document.querySelectorAll(".settings-panel")) {
    panel.hidden = panel.dataset.panel !== tab;
  }
  for (const button of document.querySelectorAll(".settings-tab")) {
    button.classList.toggle("on", button.dataset.tab === tab);
  }
}

/// Says something on a settings panel's outcome line.
function settingsSays(id, text) {
  const line = el(id);
  line.hidden = false;
  line.textContent = text;
}

/// Who this installation belongs to.
async function drawAccount() {
  const account = await invoke("account_state");
  el("account-who").textContent = account.username
    ? `signed in as ${account.username}`
    : "this installation";
  el("export-nudge").hidden = !justProtected;
}

async function drawCeiling() {
  const limit = await invoke("storage_ceiling");
  const gb = (bytes) => bytes / (1024 * 1024 * 1024);
  el("ceiling-gb").value = String(Math.max(1, Math.round(gb(limit.ceiling))));
  el("ceiling-usage").textContent =
    `Using ${gb(limit.used).toFixed(2)} GB of ${gb(limit.ceiling).toFixed(0)} GB, ` +
    `across ${limit.networks} network${limit.networks === 1 ? "" : "s"}.`;
}

for (const button of document.querySelectorAll(".settings-tab")) {
  button.addEventListener("click", async () => {
    showSettings(button.dataset.tab);
    await drawSettings();
  });
}

async function drawSettings() {
  await drawAccount();
  try {
    await drawCeiling();
  } catch (err) {
    settingsSays("ceiling-error", String(err && err.message ? err.message : err));
  }
}

el("open-settings").addEventListener("click", async () => {
  show("settings");
  showSettings("device");
  await drawSettings();
});

el("close-settings").addEventListener("click", () => show("workspace"));

// **Escape leaves too** — §4.2: settings is a full-window view, and a full-window
// view with one way out is the trap a sheet at least never was.
window.addEventListener("keydown", (event) => {
  if (event.key !== "Escape") return;
  if (el("join-sheet").open || el("create-sheet").open) return;
  if (!el("settings").hidden) show("workspace");
});

el("ceiling-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  el("ceiling-error").hidden = true;
  // Floored at one gigabyte rather than zero. A ceiling of nothing would stop
  // the application keeping anything at all, including what somebody is reading
  // — which is not a contribution setting and must not behave like one.
  const gb = Math.max(1, Math.floor(Number(el("ceiling-gb").value) || 0));
  try {
    await invoke("set_storage_ceiling", { bytes: gb * 1024 * 1024 * 1024 });
    await drawCeiling();
  } catch (err) {
    settingsSays("ceiling-error", String(err && err.message ? err.message : err));
  }
});

el("export-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  el("export-error").hidden = true;
  el("export-done").hidden = true;
  try {
    const count = await invoke("export_bundle", {
      passphrase: el("export-pass").value,
      path: el("export-path").value.trim(),
    });
    // The passphrase does not stay in the field. It is the only thing standing
    // between anybody who picks the file up and every identity in it.
    el("export-pass").value = "";
    settingsSays(
      "export-done",
      `Written. ${count === 1 ? "One network" : `${count} networks`} are in that file — ` +
        "keep it somewhere that is not this machine, and keep the passphrase somewhere " +
        "that is not the file.",
    );
  } catch (err) {
    settingsSays("export-error", String(err && err.message ? err.message : err));
  }
});

el("import-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  el("import-error").hidden = true;
  el("import-done").hidden = true;
  try {
    const done = await invoke("import_bundle", {
      passphrase: el("import-pass").value,
      path: el("import-path").value.trim(),
    });
    el("import-pass").value = "";
    // All three are said. A restore that only reported what it added would be
    // silent about the network it deliberately left alone, which is the one
    // somebody is most likely to be asking about.
    const said = [];
    if (done.added.length) said.push(`restored ${done.added.join(", ")}`);
    if (done.skipped.length) said.push(`already here, left alone: ${done.skipped.join(", ")}`);
    if (done.refused.length) said.push(`could not restore: ${done.refused.join("; ")}`);
    settingsSays("import-done", said.length ? said.join(" · ") : "that file held no networks");
    if (done.added.length) await draw();
  } catch (err) {
    settingsSays("import-error", String(err && err.message ? err.message : err));
  }
});

/// Locks the interface. The node keeps running, deliberately — `00` §6.
///
/// The shell closes the network window on this (`09` §1.12): a lock that left a
/// network's messages on screen would not be one.
el("lock-now").addEventListener("click", async () => {
  await invoke("lock");
  await gate();
});

// --- The account gate -------------------------------------------------------

/// The screen before every other one — `02` §6.3.
///
/// Two questions rather than one, because they need different answers on
/// screen: **no account** is a first run, and **a locked one** is a login. The
/// account is forced rather than offered, since one somebody can click past is
/// a preference and not the release gate `00` §5 calls it.
async function gate() {
  const account = await invoke("account_state");
  if (account.unlocked) return true;

  show("lock");
  el("first-run").hidden = account.exists;
  el("login").hidden = !account.exists;

  if (account.exists) {
    el("login-greeting").textContent = account.username
      ? `welcome back, ${account.username}`
      : "unlock this installation";
    el("login-password").focus();
  } else {
    // **Says what it is about to protect**, rather than asking for a password
    // with no reason given. An installation that predates the keyring has seeds
    // on disk in the clear, and that is the fact worth putting on screen.
    el("first-run-note").textContent =
      account.unprotected > 0
        ? `${account.unprotected === 1 ? "One network" : `${account.unprotected} networks`} on ` +
          "this machine still hold their identity unencrypted on disk. A password " +
          "wraps them, and nothing else here changes."
        : "A password wraps every identity this machine holds. It is local to this " +
          "machine and no network ever sees it.";
  }
  return false;
}

el("first-run").addEventListener("submit", async (event) => {
  event.preventDefault();
  const password = el("first-run-password").value;
  if (password !== el("first-run-again").value) {
    el("first-run-error").hidden = false;
    el("first-run-error").textContent = "those two do not match";
    return;
  }
  try {
    await invoke("create_account", {
      username: el("first-run-name").value.trim(),
      password,
    });
    // **Offered straight after, and not in the way** — `02` §6.3. A first run
    // that refused to proceed until a file had been saved somewhere is a flow
    // people learn to defeat, and the bundle protects against losing the
    // machine rather than against the next five minutes.
    justProtected = true;
    await start();
    show("settings");
    showSettings("device");
    await drawSettings();
  } catch (err) {
    el("first-run-error").hidden = false;
    el("first-run-error").textContent = String(err && err.message ? err.message : err);
  }
});

el("login").addEventListener("submit", async (event) => {
  event.preventDefault();
  try {
    await invoke("unlock", { password: el("login-password").value });
    el("login-password").value = "";
    await start();
  } catch (err) {
    el("login-error").hidden = false;
    el("login-error").textContent = String(err && err.message ? err.message : err);
  }
});

async function start() {
  if (!(await gate())) return;
  // Whichever network was open last, once there is a key to open it with. It
  // starts the nodes; it does not put a window on screen, because `09` §1.13
  // makes unlocking arrive at the list rather than at somebody's last network.
  try {
    await invoke("resume");
  } catch {
    // Nothing to resume is not a failure; the list asks.
  }
  await draw();
}

start();
