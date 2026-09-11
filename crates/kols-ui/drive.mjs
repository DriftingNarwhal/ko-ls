// Drives this interface against a stubbed shell, in a DOM with no browser.
//
// **Not a gate, and deliberately not wired into one.** This workspace's gate is
// `cargo test` and `cargo clippy`, and adding a second toolchain to it is a
// decision nobody has made. This is a tool: it needs `npm install jsdom` beside
// it and is run by hand.
//
//     cd crates/kols-ui && npm install jsdom && node drive.mjs
//
// It is here because 2,600 lines of front end had nothing asking it questions,
// and the first half hour of it having something found two real bugs — a
// first-sight mark that vanished on the next poll, and one that never cleared
// at all in a network with a single channel. Neither is visible by reading.
//
// What it cannot do is layout: jsdom applies no CSS, so nothing here says a
// panel is on screen in the place it should be. It answers the other half —
// whether the wiring runs without throwing, whether an element that should have
// been found was, and whether a class ends up on the row it belongs on.

import { JSDOM } from "jsdom";
import fs from "node:fs";

const UI = new URL(".", import.meta.url).pathname;
const html = fs.readFileSync(`${UI}/index.html`, "utf8").replace(/<script src="app.js"><\/script>/, "");
const app = fs.readFileSync(`${UI}/app.js`, "utf8");

// ── a node that answers ────────────────────────────────────────────────
const me = {
  network: "ab".repeat(32), label: "", name: "corey",
  identity: "id-corey-0001", network_name: "the workshop", has_key: true,
  may_post: true, may_create_channel: true, may_manage_channel: true,
  may_invite: true, may_moderate: true, may_set_relays: true,
  may_define_group: true, admission_mode: "intake", member_vote: false,
};
const channels = [
  { id: "c1", name: "general", topic: "everything", archived: false, private: false, position: 0 },
  { id: "c2", name: "random", topic: "", archived: false, private: false, position: 1024 },
];
let messages = [
  { id: "m1", author: "corey", author_id: "id-corey-0001", at: "10:00", at_millis: 1000, body: "one",
    edited: false, withdrawn: false, redacted: false, pinned: false, reactions: [], mine: true },
  { id: "m2", author: "sam", author_id: "id-sam-0002", at: "10:01", at_millis: 2000, body: "two",
    edited: false, withdrawn: false, redacted: false, pinned: false, reactions: [], mine: false },
];
let waiting = [];
let people = [
  {
    identity: "id-corey-0001",
    short: "id-cor",
    name: "corey",
    connected: false,
    presence: "here",
    you: true,
  },
  // No `presence` at all: nothing has been heard from them. Deliberately not
  // the string "offline", which is the claim `design/09` §4.1 forbids and which
  // this fixture would otherwise make it easy to start rendering.
  { identity: "id-sam-0002", short: "id-sam", name: "sam", connected: false, you: false },
];

const calls = [];
const answers = {
  // Unlocked by default, so the checks below are about the app rather than about
  // the gate in front of it. The gate has its own section at the end.
  account_state: () => ({ exists: true, unlocked: true, username: "corey", unprotected: 0 }),
  resume: () => true,
  create_account: () => 0,
  export_bundle: () => 1,
  import_bundle: () => ({ added: [], skipped: [], refused: [] }),
  unlock: () => 0,
  lock: () => null,
  me: () => me,
  sidebar: () => channels.map((channel) => ({ kind: "channel", channel })),
  open_channel: ({ channel }) => ({
    channel, messages, authors: 2, refused: [],
    // The range `design/09` §4.4 describes: this fixture holds the whole
    // channel, so there is nothing either side and it runs to the tail.
    oldest: "cursor-oldest", newest: null, older: false, newer: false,
    more_history: false,
  }),
  people: () => people,
  waiting: () => waiting,
  relays: () => ({ designated: [], live: [], cached: [], standing: "none", detail: "" }),
  networks: () => [],
  reorg: () => null,
  settings: () => [],
  roles: () => [],
  contribution: () => offer,
  set_presence: (args) => {
    const you = people.find((person) => person.you);
    you.presence = args.state;
    return null;
  },
  storage_ceiling: () => ceiling,
  set_storage_ceiling: (args) => {
    ceiling = { ...ceiling, ceiling: args.bytes };
    return null;
  },
  start_conversation: (args) => {
    started.push(args);
    return "ee".repeat(32);
  },
  set_contribution: (args) => {
    saved.push(args);
    return null;
  },
  // What another of this member's networks already designates — D29. The
  // workspace answers this for real; here it is whatever the check under test
  // needs it to be.
  shared_relays: () => sharedRelays,
  // The count the interface hands the shell, which composes the title from it
  // and from replayed state — D36, `design/09` §1.3.
  set_unread: ({ unread }) => { unreadSent.push(unread); return null; },
  show_workspace: () => null,
  shared_relays_for_new_network: () => sharedRelays,
  set_relays: () => null,
  create_network: () => ({ id: "cd".repeat(32), label: "second", open: false }),
};

// What `contribution` currently answers, and what `set_contribution` was asked
// for — the MiB/bytes conversion happens in the interface, so it is the half a
// test can actually be wrong about.
let offer = {
  storage_offered: 256 * 1024 * 1024,
  upload_offered: 1_000_000,
  download_offered: 8_000_000,
  relay_willing: false,
  is_default: true,
  storage_used: 32 * 1024 * 1024,
  storage_total: 200 * 1024 * 1024,
  reachable: null,
};
const saved = [];
const started = [];
const unreadSent = [];
const asked = [];
// Empty unless a check is exercising the shared-relay warning.
let sharedRelays = [];
let ceiling = { ceiling: 2 * 1024 * 1024 * 1024, used: 1_288_490_188, networks: 3 };

const dom = new JSDOM(html, { runScripts: "outside-only", pretendToBeVisual: true, url: "http://localhost/" });
const { window } = dom;
const listeners = {};
const titles = [];
window.__TAURI__ = {
  core: {
    invoke: async (name, args) => {
      calls.push(name);
      const answer = answers[name];
      if (!answer) throw new Error(`no stub for ${name}`);
      return answer(args ?? {});
    },
  },
  event: { listen: async (name, run) => { listeners[name] = run; return () => {}; } },
  window: {
    getCurrentWindow: () => ({
      setTitle: async (t) => titles.push(t),
      isFocused: async () => false,
      requestUserAttention: async () => {},
    }),
  },
};
window.localStorage.clear();

// jsdom implements no `confirm`, so until now every confirmation in this
// interface resolved falsy and its guarded action was skipped. That is preserved
// deliberately: `answering` defaults to false, so no check below changes meaning
// because this stub arrived. A check that is *about* a confirmation sets it.
let answering = false;
const confirmed = [];
window.confirm = (message) => {
  confirmed.push(message);
  return answering;
};

const problems = [];
window.addEventListener("error", (e) => problems.push(`error: ${e.error?.stack ?? e.message}`));
window.addEventListener("unhandledrejection", (e) => problems.push(`rejected: ${e.reason?.stack ?? e.reason}`));
process.on("unhandledRejection", (reason) => problems.push(`rejected: ${reason?.stack ?? reason}`));

window.eval(app);

const el = (id) => window.document.getElementById(id);
const settled = () => new Promise((done) => setTimeout(done, 60));
const say = (name, ok, detail = "") =>
  console.log(`${ok ? "  ok  " : "FAIL  "}${name}${detail ? " — " + detail : ""}`);

await settled();

// ── the app came up ────────────────────────────────────────────────────
say("app view is showing", !window.document.querySelector(".app").hidden);
say("settings is not", el("settings").hidden);
// A joiner has no local label at all, so the network's own name is the only one
// there is — this used to render as "unnamed network".
say("the network's own name is drawn", el("network-label").textContent === "the workshop",
    el("network-label").textContent);
say("network id is a hover", el("network-label").title.includes(me.network), el("network-label").title.slice(0, 20));
say("identity is a hover", el("you-line").title === "id-corey-0001");
say("door offered to an inviter", !el("open-door").hidden);
say("presence is on screen", !el("presence").hidden);

// ── the channel row's menu is on a right-click, and only there ─────────
const row = el("channel-list").querySelector(".channel-item > button");
// One button per row, and no second way into the menu beside it.
say("each row carries nothing but the channel",
    [...el("channel-list").querySelectorAll(".channel-item")]
      .every((item) => item.querySelectorAll("button").length === 1));
row?.dispatchEvent(
  new window.MouseEvent("contextmenu", { bubbles: true, clientX: 10, clientY: 10 }),
);
await settled();
const menu = window.document.querySelector(".pop-menu");
say("right-click opens the menu", Boolean(menu));
const entries = [...(menu?.querySelectorAll("button") ?? [])].map((b) => b.textContent);
say("menu offers rename and delete", entries.includes("rename") && entries.includes("delete"), entries.join(", "));
// Reordering has a route that does not depend on the webview starting a drag,
// which it has never been seen to do.
say("the first channel offers move down and not move up",
    entries.includes("move down") && !entries.includes("move up"), entries.join(", "));
menu?.remove();

// And the move actually reorders: the second channel, moved up, lands before the
// first.
const second = [...el("channel-list").querySelectorAll(".channel-item")][1];
second.querySelector("button").dispatchEvent(
  new window.MouseEvent("contextmenu", { bubbles: true, clientX: 10, clientY: 10 }),
);
await settled();
const up = [...window.document.querySelectorAll(".pop-menu button")].find(
  (b) => b.textContent === "move up",
);
say("the second channel offers move up", Boolean(up));
calls.length = 0;
up?.dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
await settled();
say("moving up asks the core to move it", calls.includes("move_channel"), calls.join(", "));
window.document.querySelector(".pop-menu")?.remove();

// The drag path is wired correctly even though nothing has been seen to start
// one — worth holding, since the difference between "not wired" and "never
// begun" is the difference between fixing this and removing it.
const sidebarRows = [...el("channel-list").querySelectorAll(".channel-item")];
say("the row itself is draggable, not the button inside it",
    sidebarRows[0].draggable === true &&
      sidebarRows[0].querySelector("button").draggable === false);
const carry = { setData() {}, effectAllowed: "", dropEffect: "" };
// jsdom lays nothing out, so every box is zero-sized and `landsAfter` compares
// against a midpoint of zero. `clientY` decides which half regardless, which is
// what these are actually asserting.
const fire = (type, target, clientY = -1) => {
  const event = new window.Event(type, { bubbles: true, cancelable: true });
  event.dataTransfer = carry;
  event.clientY = clientY;
  target.dispatchEvent(event);
  return event;
};
fire("dragstart", sidebarRows[1]);
say("dragging over a row offers a drop", fire("dragover", sidebarRows[0]).defaultPrevented);
say("the upper half marks before",
    sidebarRows[0].classList.contains("drop-before"), sidebarRows[0].className);
fire("dragover", sidebarRows[0], 1);
say("the lower half marks after",
    sidebarRows[0].classList.contains("drop-after"), sidebarRows[0].className);
calls.length = 0;
fire("drop", sidebarRows[0]);
await settled();
say("dropping asks the core to move it", calls.includes("move_channel"), calls.join(", "));

// Dropping a channel on itself is not a move — it used to resolve to "before
// nothing" and land the channel at the top of the list.
fire("dragstart", sidebarRows[0]);
calls.length = 0;
fire("drop", sidebarRows[0]);
await settled();
say("dropping a channel on itself does nothing", !calls.includes("move_channel"),
    calls.join(", ") || "(nothing)");

// The way out of a folder, which is the only way once every channel is in one.
const nav = window.document.querySelector(".channels");
fire("dragstart", sidebarRows[1]);
say("the sidebar's empty space takes a drop", fire("dragover", nav).defaultPrevented);
say("and says it means the top level", nav.classList.contains("drop-into"));
calls.length = 0;
fire("drop", nav);
await settled();
say("dropping there moves it out", calls.includes("move_channel"), calls.join(", "));

// ── who is here ────────────────────────────────────────────────────────
say("count starts at zero", el("presence-count").textContent === "0");
say("own dot unlit with nobody connected", !el("me-dot").classList.contains("live"));
people = [people[0], { ...people[1], connected: true }];
await window.eval("drawPeople()");
await settled();
say("count follows a connection", el("presence-count").textContent === "1", el("presence-count").textContent);
say("own dot lights", el("me-dot").classList.contains("live"));
say("roster hidden until asked", el("presence-panel").hidden);
el("presence-toggle").dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
await settled();
say("clicking opens the roster", !el("presence-panel").hidden);
say("roster lists both", el("roster-list").children.length === 2);
window.document.dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
await settled();
say("clicking away closes it", el("presence-panel").hidden);

// ── the door ───────────────────────────────────────────────────────────
say("door sheet starts closed", el("door").hidden);
waiting = [{ identity: "id-new", short: "id-new" }];
await window.eval("drawWaiting()");
await settled();
say("waiting shows on the button", !el("door-count").hidden && el("door-count").textContent === "1");
el("open-door").dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
await settled();
say("the button opens the sheet", !el("door").hidden);
say("the sheet lists who is waiting", el("waiting-list").children.length === 1);

// ── settings is a screen, not a layer ──────────────────────────────────
el("close-door").dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
el("open-settings").dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
await settled();
say("settings takes the window", !el("settings").hidden && window.document.querySelector(".app").hidden);
window.document.dispatchEvent(new window.KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
await settled();
say("escape gives it back", el("settings").hidden && !window.document.querySelector(".app").hidden);

// ── first sight of a message ───────────────────────────────────────────
// **`.message`, not every child.** The list also carries a notice about the list
// — where history stops, or that this is the start of the channel — which is
// deliberately not a message so that anything selecting `.message` cannot find
// it. This read every child, so adding the notice shifted every index by one and
// four checks failed for a reason that had nothing to do with marks. The app's
// own comment warned that a row which looks like a message and is not one is
// fine "until the day something counts them"; this was the thing counting them.
const rows = () =>
  [...el("messages").querySelectorAll(".message")].map((r) => r.classList.contains("fresh"));
say("first visit highlights nothing", rows().every((f) => !f), JSON.stringify(rows()));
messages = [
  messages[0],
  { id: "m3", author: "sam", author_id: "id-sam-0002", at: "10:00:30", at_millis: 1500, body: "late",
    edited: false, withdrawn: false, redacted: false, pinned: false, reactions: [], mine: false },
  messages[1],
];
await listeners["kols://records"]({ payload: [null, "c1", true] });
await settled();
say("a message landing mid-timeline is marked", JSON.stringify(rows()) === "[false,true,false]", JSON.stringify(rows()));
await window.eval("refresh()");
await settled();
say("the mark survives a redraw", JSON.stringify(rows()) === "[false,true,false]", JSON.stringify(rows()));

// A redraw under a reader who has not moved must keep it; arriving again clears it.
await listeners["kols://records"]({ payload: [null, "c1", true] });
await settled();
say("another arrival keeps the earlier mark", JSON.stringify(rows()) === "[false,true,false]", JSON.stringify(rows()));
el("channel-list").querySelector("button").dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
await settled();
say("clicking the channel clears them", rows().every((f) => !f), JSON.stringify(rows()));

// ── an event from a network this window is not showing ─────────────────
//
// Several nodes run at once now (`design/09` §2), so events arrive from
// networks nobody has open. One of those must not redraw or mark the network on
// screen — a message in a conversation would otherwise count against a server's
// unread and reopen its channel.
//
// `state` is not reachable from here (app.js runs under an indirect eval, so its
// `const` bindings are not global) so this drives the real path — which is now
// the shell telling this window which network it is drawing. A network window
// is reused as the member switches (`design/09` §1.5), so that event is how it
// finds out, and driving it here exercises the path rather than a stand-in.
answers.open_network = () => null;
await listeners["kols://network"]({ payload: "aa11bb22" });
await settled();

// Observed through the redraw the handler performs, not through the rows it
// leaves behind: an unguarded foreign event re-opens the current channel, and
// comparing rendered rows would miss that because it redraws the same content.
// A first attempt did exactly that and survived deleting the guard.
const drewFor = [];
const drawChannel = answers.open_channel;
answers.open_channel = (args) => { drewFor.push(args?.channel ?? null); return drawChannel(args); };

await listeners["kols://records"]({ payload: ["ffffffff", "c2", true] });
await settled();
say("an event from another network reaches nothing here",
    drewFor.length === 0,
    JSON.stringify(drewFor));

// And the guard is not simply off for everything.
await listeners["kols://records"]({ payload: ["aa11bb22", "c1", true] });
await settled();
say("and one from the network in view still arrives", drewFor.length > 0, JSON.stringify(drewFor));
answers.open_channel = drawChannel;

// ── what a mark is not for ─────────────────────────────────────────────
messages = [
  ...messages,
  { id: "m4", author: "corey", author_id: "id-corey-0001", at: "10:02", at_millis: 3000, body: "mine",
    edited: false, withdrawn: false, redacted: false, pinned: false, reactions: [], mine: true },
  { id: "m5", author: "sam", author_id: "id-sam-0002", at: "10:03", at_millis: 4000, body: "theirs",
    edited: false, withdrawn: false, redacted: false, pinned: false, reactions: [], mine: false },
];
await listeners["kols://records"]({ payload: [null, "c1", true] });
await settled();
say("your own message is never marked", JSON.stringify(rows()) === "[false,false,false,false,true]",
    JSON.stringify(rows()));

// Hovering it is reading it.
const marked = [...el("messages").querySelectorAll(".message")].find((r) => r.classList.contains("fresh"));
marked?.dispatchEvent(new window.MouseEvent("mouseenter", { bubbles: false }));
await settled();
say("hovering a marked message clears it", rows().every((f) => !f), JSON.stringify(rows()));

// ── being told from outside the window ─────────────────────────────────
//
// **The count reaches the shell rather than the title.** D36 keeps the network
// and the identity out of the themeable document, and a title the *document*
// set would be one the document could set wrongly — so the shell composes it
// from replayed state and this supplies only the number (`design/09` §1.3).
// What is asserted is therefore the call, not the string: a document that could
// produce the string would be the thing D36 forbids.
await listeners["kols://records"]({ payload: [null, "c2", true] });
await settled();
say("unread reaches the shell, which owns the title",
    unreadSent.at(-1) === 1, JSON.stringify(unreadSent.at(-1)));
say("and this document never sets a title itself",
    titles.length === 0, JSON.stringify(titles));

// ── O21: a node without a key is in one of two different places ────────
//
// The interface used to say one thing for both, and send a member who was
// already admitted off to find an admin who had nothing left to do. Membership
// comes from replayed governance rather than from what the join handshake said,
// which is what makes the middle case reachable at all.
window.drawKeyState({ has_key: true, is_member: true });
say("a keyed member is told nothing", el("key-state").textContent === "",
    JSON.stringify(el("key-state").textContent));

window.drawKeyState({ has_key: false, is_member: false });
say("an unadmitted member is told to wait for one",
    el("key-state").textContent.includes("waiting to be admitted"),
    el("key-state").textContent);

window.drawKeyState({ has_key: false, is_member: true });
say("an admitted member is told it resolves itself, not to chase somebody",
    el("key-state").textContent.includes("admitted, waiting to be keyed in") &&
      !el("key-state").textContent.includes("until a member admits you"),
    el("key-state").textContent);

// ── contribution: an offer to others, never this member's own working set ──
await window.drawContribution();
say("storage is shown in megabytes", el("contribution-storage").value === "256",
    el("contribution-storage").value);
say("bandwidth is shown in KB/s",
    el("contribution-upload").value === "1000" && el("contribution-download").value === "8000",
    `${el("contribution-upload").value} / ${el("contribution-download").value}`);
say("an unset offer says it is riding the defaults", !el("contribution-default").hidden);

// The two numbers stay two numbers. Reporting one would either overstate what a
// member gives or understate what the application costs them.
say("what is held for others is reported",
    el("contribution-usage").textContent.includes("Holding 32 MB for other members"),
    el("contribution-usage").textContent);
say("and the disk total is reported separately, as not a contribution",
    el("contribution-usage").textContent.includes("200 MB on this disk") &&
      el("contribution-usage").textContent.includes("not a contribution"));

// Relaying is offered only where it could work. A node nobody has seen from
// outside cannot help two members reach each other, so the control is absent
// and the reason is on screen rather than left to be guessed.
say("an unreachable node is not offered relaying", el("contribution-relay-row").hidden);
say("and is told why", !el("contribution-unreachable").hidden);

offer = { ...offer, is_default: false, reachable: "/ip4/203.0.113.7/tcp/4001" };
await window.drawContribution();
say("a chosen offer does not claim to be the default", el("contribution-default").hidden);
say("a reachable node is offered relaying", !el("contribution-relay-row").hidden);
say("and the reason is withdrawn", el("contribution-unreachable").hidden);

el("contribution-storage").value = "512";
el("contribution-upload").value = "250";
el("contribution-relay").checked = true;
el("contribution-form").dispatchEvent(
  new window.Event("submit", { bubbles: true, cancelable: true }),
);
await settled();
say("saving converts each unit to bytes",
    saved.at(-1).storageOffered === 512 * 1024 * 1024 && saved.at(-1).uploadOffered === 250_000,
    JSON.stringify(saved.at(-1)));
say("and carries the relay choice", saved.at(-1).relayWilling === true);

// Zero is a real answer everywhere and must survive the round trip rather than
// being read as "nothing entered" and replaced by a default.
el("contribution-storage").value = "0";
el("contribution-upload").value = "0";
el("contribution-form").dispatchEvent(
  new window.Event("submit", { bubbles: true, cancelable: true }),
);
await settled();
say("zero is saved as zero, not as unset",
    saved.at(-1).storageOffered === 0 && saved.at(-1).uploadOffered === 0,
    JSON.stringify(saved.at(-1)));

// A node that stopped being reachable must not keep sending a claim it can no
// longer back up, even with a checkbox left ticked from before.
offer = { ...offer, reachable: null };
await window.drawContribution();
el("contribution-relay").checked = true;
el("contribution-form").dispatchEvent(
  new window.Event("submit", { bubbles: true, cancelable: true }),
);
await settled();
say("an unreachable node never claims to relay", saved.at(-1).relayWilling === false);

// ── a bounded channel says so, and is not mistaken for a message ───────
//
// A channel the ceiling stopped filling renders exactly like a quiet one, and
// only one of them is worth telling somebody about.
answers.open_channel = ({ channel }) => ({
  channel, messages, authors: 2, refused: [], more_history: true,
});
await listeners["kols://records"]({ payload: [null, "c1", true] });
await settled();
const notice = el("messages").querySelector('[data-kols="more-history"]');
say("a bounded channel says so at the top",
    notice !== null && notice.textContent.includes("not lost"),
    notice ? notice.textContent.slice(0, 60) : "(absent)");
// Asking is bounded by the oldest message on screen — the point where this
// member's view stops — and reports what it did rather than pretending to hold
// the page, since the records arrive later as an event.
answers.fetch_history = (args) => {
  asked.push(args);
  return null;
};
// Guarded: this used to dereference `notice` directly, so any change that
// stopped the notice rendering aborted the whole run at this line and hid every
// check after it. A driver that stops reporting on the first surprise is worth
// less than the surprise it found.
const ask = notice?.querySelector('[data-kols="fetch-history"]');
if (!ask) say("the fetch control is on the notice", false, "(no notice to carry it)");
ask?.dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
await settled();
say("asking is bounded by the oldest message on screen",
    asked.at(-1)?.beforeMillis === messages[0].at_millis,
    JSON.stringify(asked.at(-1)));
say("and says it asked rather than claiming to have fetched",
    Boolean(ask?.disabled) && !ask?.textContent.includes("fetched"), ask?.textContent);

say("and the notice is not a message",
    notice !== null && !notice.classList.contains("message"));
// The marks select `.message.fresh`, so the notice must be reachable by neither
// half of that. Asserted against the live document rather than by reading the
// class string, since the selector is what actually decides.
say("so the first-sight marks cannot reach it",
    ![...el("messages").querySelectorAll(".message")].includes(notice) &&
      !notice?.classList.contains("fresh"));

// The installation-wide ceiling moved to the workspace window with the rest of
// *this device* (`design/09` §1.13), and is checked in the second pass below.

// ── presence, and the word that must never appear ──────────────────────
await window.drawPeople();
const rosterRows = () => [...el("roster-list").querySelectorAll(".person")];
say("a member who said something has it shown beside their name",
    rosterRows()[0].querySelector(".said")?.textContent === "here");
// **The whole of §4.1.** Nothing has been heard from sam, which is not a claim
// that sam is away — so the row carries no word at all rather than a wrong one.
say("a member nothing has been heard from is not labelled",
    rosterRows()[1].querySelector(".said") === null);
say("and the roster never says offline",
    !/offline/i.test(el("roster-list").textContent + el("roster-note").textContent),
    el("roster-note").textContent);
say("the note explains both marks",
    /lit dot/i.test(el("roster-note").textContent) &&
      /nothing has been heard/i.test(el("roster-note").textContent));

// Choosing invisible has to say what it actually does, because the failure
// worth avoiding is somebody believing they are hidden while a beat goes out.
el("my-presence").value = "invisible";
el("my-presence").dispatchEvent(new window.Event("change", { bubbles: true }));
await settled();
say("choosing invisible reaches the shell with that exact value",
    people.find((person) => person.you).presence === "invisible");
say("and the window says nothing at all is published",
    !el("my-presence-note").hidden &&
      /not even that you are hiding/i.test(el("my-presence-note").textContent),
    el("my-presence-note").textContent);
say("the selector shows what is published rather than what was clicked",
    el("my-presence").value === "invisible");

// ── starting a conversation, which has no directory — §1.7 ─────────────
//
// **The roster is the entry point and there is no name box**, because a request
// binds one identity in one network (spec 07 §6.2) and a roster row is already
// that pair. A field that took a name could not work: names are per network,
// are not unique and are not identifiers, and a cross-network one would be a
// phishing surface whose attacker's half is typing.
say("no window in this client offers to find somebody by name",
    !/add (a )?friend|find (a )?(user|person)|search for somebody/i.test(
      window.document.body.textContent + fs.readFileSync(`${UI}/workspace.html`, "utf8")));

await window.drawPeople();
const sam = rosterRows().find((r) => r.textContent.includes("sam"));
sam.dispatchEvent(new window.MouseEvent("click", { bubbles: true, clientX: 10, clientY: 10 }));
await settled();
const offerEntries = [...window.document.querySelectorAll(".pop-menu button")].map(
  (b) => b.textContent,
);
say("a roster row offers to start a conversation", offerEntries.join(",").includes("message sam"),
    JSON.stringify(offerEntries));

started.length = 0;
[...window.document.querySelectorAll(".pop-menu button")]
  .find((b) => b.textContent.includes("message"))
  .dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
await settled();
say("and it asks with the identity in this network, not the name",
    started.length === 1 && started[0].with === "id-sam-0002" && started[0].network === "ab".repeat(32),
    JSON.stringify(started));

// **Never *waiting for an answer*** (`09` §1.8), and not drawn as a failure
// either — a request sent is the flow working.
const toldLine = el("app-error");
say("what it says is what the other side will see, and not a wait",
    /both online/i.test(toldLine.textContent) && !/waiting/i.test(toldLine.textContent),
    toldLine.textContent.slice(0, 70));
say("and a request sent is not drawn as a refusal",
    toldLine.classList.contains("told"));

// Your own row is not an entry point: `dm::start` would refuse it, and a menu
// that offered it would be a control that cannot work.
window.document.querySelector(".pop-menu")?.remove();
rosterRows()
  .find((r) => r.textContent.includes("(you)"))
  .dispatchEvent(new window.MouseEvent("click", { bubbles: true, clientX: 10, clientY: 10 }));
await settled();
say("your own row offers nothing", window.document.querySelector(".pop-menu") === null);

// ── what the contribution panel promises ───────────────────────────────
// **A shipped sentence that contradicts the code is worse than no sentence.**
// This panel told members storage was "an offer rather than a ceiling" and that
// the node "takes on no storage duty" for a day after both had stopped being
// true — the number was a hard cap and the node was holding replicas under it.
// Read off the live document, so the guard breaks when the copy drifts back.
const contribution = window.document
  .querySelector('[data-panel="contribution"]')
  .textContent.replace(/\s+/g, " ");
say("the contribution panel does not tell members the cap is unenforced",
    !/(offer|rather than) a ceiling|nothing enforces|no storage duty|being built/i
      .test(contribution));
say("and it says what offering more actually buys",
    /short of/i.test(contribution) && /whichever is smaller/i.test(contribution));

// ── a relay two of one member's networks would share — D29, O11 ────────
// The client is the only party that can see this: the relay replays no log and
// neither network's members can see across the two. So the check here is that
// the question is *asked before* the designation rather than reported after it,
// and that answering no means nothing was written.
const RELAY = "/dns4/relay.example/tcp/443/p2p/12D3KooWAT1R2JjcZbnVUKLX8Xo1Qg5APTWMkpHarHY4Uo1YpGzT";

sharedRelays = [];
confirmed.length = 0;
calls.length = 0;
el("relay-input").value = RELAY;
el("relay-form").dispatchEvent(new window.Event("submit", { bubbles: true, cancelable: true }));
await settled();
say("a relay nobody else uses is designated without a word",
    confirmed.length === 0 && calls.includes("set_relays"));

sharedRelays = [{ relay: RELAY, label: "the workshop", id: "ab".repeat(32) }];
answering = false;
confirmed.length = 0;
calls.length = 0;
el("relay-input").value = RELAY;
el("relay-form").dispatchEvent(new window.Event("submit", { bubbles: true, cancelable: true }));
await settled();
say("a shared relay is raised before anything is written", confirmed.length === 1);
say("and the warning names the other network and what it costs",
    /the workshop/.test(confirmed[0] ?? "") && /discover each other/i.test(confirmed[0] ?? ""),
    confirmed[0]);
// The half that makes it a decision rather than a notification.
say("declining designates nothing", !calls.includes("set_relays"));

answering = true;
calls.length = 0;
el("relay-input").value = RELAY;
el("relay-form").dispatchEvent(new window.Event("submit", { bubbles: true, cancelable: true }));
await settled();
say("and it warns rather than refuses — agreeing goes through",
    calls.includes("set_relays"));

// Creating a network with a relay is a designation too, and it is the first one
// most people make — it moved to the workspace window with the act itself, and
// is checked there (the second pass at the end of this file).

// ── paging: the loaded range ───────────────────────────────────────────
//
// jsdom applies no layout, so `scrollTop`, `clientHeight` and `scrollHeight` are
// all zero here and no amount of driving can produce a real scroll. That is why
// `design/09` §4.4 requires the range to be pure state fed numbers by the DOM
// layer: everything below is answerable without a viewport, and it is the half
// that carries the rules.

console.log("\n── paging ──");

const w = (js) => window.eval(js);

// Nothing loaded is how "give me the newest page" is expressed. It must send no
// cursor at all rather than inventing one — this file never builds one.
const fresh = w("JSON.stringify(windowFor(null))");
say("an empty range asks for a page and names no cursor",
    JSON.parse(fresh).back === 50 && JSON.parse(fresh).oldest === undefined, fresh);

// The tick asks for the range it holds, reaching nowhere.
const ticking = JSON.parse(w('JSON.stringify(windowFor({ oldest: "o", newest: null }))'));
say("a tick re-reads the held range and reaches nowhere",
    ticking.oldest === "o" && ticking.newest === null && ticking.back === 0,
    JSON.stringify(ticking));

// A reach names the same range and asks for one page more.
const reaching = JSON.parse(w('JSON.stringify(windowFor({ oldest: "o", newest: null }, { back: 50 }))'));
say("a reach names the same range and asks for one page more",
    reaching.oldest === "o" && reaching.back === 50, JSON.stringify(reaching));

say("reaching the top is a scroll position, not a guess",
    w("reachedTop({ scrollTop: 0, clientHeight: 100, scrollHeight: 1000 })") === true);
say("and part way down is not",
    w("reachedTop({ scrollTop: 900, clientHeight: 100, scrollHeight: 1000 })") === false);
// The case that would otherwise walk a whole channel backwards with nobody
// touching it: a list shorter than its container is always at the top.
say("a list too short to scroll never asks for more",
    w("reachedTop({ scrollTop: 0, clientHeight: 500, scrollHeight: 400 })") === false);

// ── the three top-of-list states ───────────────────────────────────────
//
// Three because they are three different promises: a disk read that loads
// itself, a network round trip that may not answer, and the start of the
// conversation. The middle one must never be shown above undrawn local history,
// which would be the interface saying this machine is not holding what it holds.
const topRow = () => {
  const first = el("messages").firstElementChild;
  return first?.dataset?.kols ?? "(none)";
};

answers.open_channel = ({ channel }) => ({
  channel, messages, authors: 2, refused: [],
  oldest: "o", newest: null, older: true, newer: false, more_history: true,
});
await w('openChannel("c1")');
await settled();
say("local history outranks the network notice",
    topRow() === "older-local"
      && el("messages").querySelector('[data-kols="more-history"]') === null,
    topRow());

// **And the branch is keyed on the local boundary, not on the network one.**
// Without this case the check above passes even when the two conditions are
// swapped, because the fixture sets both — which is how it read for its first
// half hour of existence.
answers.open_channel = ({ channel }) => ({
  channel, messages, authors: 2, refused: [],
  oldest: "o", newest: null, older: true, newer: false, more_history: false,
});
await w('openChannel("c1")');
await settled();
say("undrawn local history shows on its own, with no network boundary at all",
    topRow() === "older-local", topRow());

answers.open_channel = ({ channel }) => ({
  channel, messages, authors: 2, refused: [],
  oldest: "o", newest: null, older: false, newer: false, more_history: true,
});
await w('openChannel("c1")');
await settled();
say("with nothing local left, the network notice is what shows",
    topRow() === "more-history", topRow());

answers.open_channel = ({ channel }) => ({
  channel, messages, authors: 2, refused: [],
  oldest: "o", newest: null, older: false, newer: false, more_history: false,
});
await w('openChannel("c1")');
await settled();
say("and a whole channel says where it begins",
    topRow() === "channel-start", topRow());

// ── the range is sent back ─────────────────────────────────────────────
//
// The property the whole model rests on: a tick must ask about the range that
// is loaded, not about the newest page, or backfill landing in the past is
// correctly ordered and permanently invisible.
let asks = [];
answers.open_channel = (args) => {
  asks.push(args);
  return {
    channel: args.channel, messages, authors: 2, refused: [],
    oldest: "held-oldest", newest: null, older: true, newer: false, more_history: false,
  };
};
await w('openChannel("c1")');
await settled();
say("opening a channel sends no range",
    asks.length === 1 && asks[0].window?.oldest === undefined,
    JSON.stringify(asks[0]?.window));

asks = [];
await w("refresh()");
await settled();
const sent = asks.find((a) => a.channel === "c1");
say("and the next read names the range that was drawn",
    sent?.window?.oldest === "held-oldest", JSON.stringify(sent?.window));

// A range from the channel somebody just left names positions that do not exist
// in the one they moved to, and would be answered with the newest page — which
// would read as a jump. Produced by answering with a *different* channel than
// was asked for, which is the same disagreement a tick landing after somebody
// moved creates, and the only one reachable from out here.
answers.open_channel = (args) => {
  asks.push(args);
  return {
    channel: "c2", messages, authors: 2, refused: [],
    oldest: "belongs-to-c2", newest: null, older: false, newer: false, more_history: false,
  };
};
await w('openChannel("c1")');
await settled();
say("a range is held for the channel it describes and no other",
    w("heldRange()") === null, JSON.stringify(w("heldRange()")));

asks = [];
await w("refresh()");
await settled();
const other = asks.find((a) => a.channel === "c1");
say("so the next read names no range rather than one from elsewhere",
    other !== undefined && other.window?.oldest === undefined,
    JSON.stringify(other?.window));

// ── marks: scrolling back is navigation, not arrival ───────────────────
//
// §4.4 takes §4.3's first-sight rule down one level. Without this, scrolling
// back on the second visit sets every old message alight.
answers.open_channel = ({ channel }) => ({
  channel, messages, authors: 2, refused: [],
  oldest: "o", newest: null, older: true, newer: false, more_history: false,
});
// A first visit files what is there and marks nothing, so the second visit has
// something to compare against.
await w('openChannel("c1")');
await settled();
await w('openChannel("c1")');
await settled();

const earlier = [
  { id: "m0", author: "sam", author_id: "id-sam-0002", at: "09:00", at_millis: 500,
    body: "long ago", edited: false, withdrawn: false, redacted: false, pinned: false,
    reactions: [], mine: false },
  ...messages,
];
answers.open_channel = ({ channel }) => ({
  channel, messages: earlier, authors: 2, refused: [],
  // Still more behind it, or the second reach below is refused by the guard and
  // this file would be testing that nothing happens twice.
  oldest: "further", newest: null, older: true, newer: false, more_history: false,
});
await w("reachBack()");
await settled();
const afterReach = el("messages").querySelectorAll(".message.fresh").length;
say("a page reached by scrolling back marks nothing",
    afterReach === 0, `${afterReach} marked`);
say("and it was drawn — the reach is not a no-op",
    el("messages").querySelectorAll(".message").length === earlier.length,
    `${el("messages").querySelectorAll(".message").length} rows`);

// The other half, and the reason the split is on the previous tail rather than
// on the reach alone: a message arriving in the same breath as a reach must
// still be marked, or nothing ever comes back for it.
const both = [...earlier, {
  id: "m9", author: "sam", author_id: "id-sam-0002", at: "10:05", at_millis: 9000,
  body: "just now", edited: false, withdrawn: false, redacted: false, pinned: false,
  reactions: [], mine: false,
}];
answers.open_channel = ({ channel }) => ({
  channel, messages: both, authors: 2, refused: [],
  oldest: "further", newest: null, older: false, newer: false, more_history: false,
});
await w("reachBack()");
await settled();
const fresh9 = [...el("messages").querySelectorAll(".message.fresh")].length;
say("but a message that arrived during the reach is still marked",
    fresh9 === 1, `${fresh9} marked`);



// ══ the workspace window ═══════════════════════════════════════════════
//
// A second document (`design/09` §1.1, D40). The list of everything this
// installation belongs to, the account that gates it, and the settings that are
// about no network in particular — all of which used to share one document with
// the network being drawn, and now do not.
//
// Driven in its own DOM rather than by reusing the one above, because that is
// what the shell does: two windows, two documents, and a check that ran them in
// one would be checking something this application does not do.

console.log("\n══ the workspace window ══");

const wsHtml = fs
  .readFileSync(`${UI}/workspace.html`, "utf8")
  .replace(/<script src="workspace.js"><\/script>/, "");
const wsSource = fs.readFileSync(`${UI}/workspace.js`, "utf8");

const wsDom = new JSDOM(wsHtml, { runScripts: "outside-only", pretendToBeVisual: true, url: "http://localhost/" });
const wsWindow = wsDom.window;
let conversationsAnswer = [];
let networksAnswer = [
  { id: "aa".repeat(32), label: "the workshop", keyed: true, open: false },
  { id: "bb".repeat(32), label: "book club", keyed: false, open: false },
];
const wsCalls = [];
const wsListeners = {};
const wsAnswers = {
  account_state: () => ({ exists: true, unlocked: true, username: "corey", unprotected: 0 }),
  resume: () => true,
  networks: () => networksAnswer,
  open_network: () => null,
  forget_network: () => ({ announced: true, reached: 2, reason: "" }),
  shared_relays_for_new_network: () => sharedRelays,
  create_network: () => ({ id: "cd".repeat(32), label: "second", open: true }),
  join_network: () => ({ admitted: true, identity: "", answered: true }),
  storage_ceiling: () => ceiling,
  set_storage_ceiling: (args) => {
    ceiling = { ...ceiling, ceiling: args.bytes };
    return null;
  },
  export_bundle: () => 2,
  import_bundle: () => ({ added: [], skipped: [], refused: [] }),
  create_account: () => null,
  unlock: () => 0,
  lock: () => null,
  conversations: () => conversationsAnswer,
  accept_conversation: () => "ee".repeat(32),
  decline_conversation: () => null,
  open_conversation: () => null,
};
wsWindow.__TAURI__ = {
  core: {
    invoke: async (name, args) => {
      wsCalls.push(name);
      const answer = wsAnswers[name];
      if (!answer) throw new Error(`no stub for ${name}`);
      return answer(args ?? {});
    },
  },
  event: {
    listen: async (name, run) => {
      wsListeners[name] = run;
      return () => {};
    },
  },
  window: { getCurrentWindow: () => ({ setTitle: async () => {}, isFocused: async () => false }) },
};
let wsAnswering = false;
const wsConfirmed = [];
wsWindow.confirm = (message) => { wsConfirmed.push(message); return wsAnswering; };
wsWindow.HTMLDialogElement.prototype.showModal = function () { this.open = true; };
wsWindow.HTMLDialogElement.prototype.close = function () { this.open = false; };
wsWindow.addEventListener("error", (e) => problems.push(`workspace error: ${e.error?.stack ?? e.message}`));
wsWindow.addEventListener("unhandledrejection", (e) => problems.push(`workspace rejected: ${e.reason?.stack ?? e.reason}`));
wsWindow.eval(wsSource);

const wsEl = (id) => wsWindow.document.getElementById(id);
await settled();

// The list, which is the whole point of the window.
say("the workspace lists what this installation belongs to",
    wsEl("networks").querySelectorAll("li").length === 2,
    `${wsEl("networks").querySelectorAll("li").length} rows`);
say("and a network not yet keyed into says so rather than looking broken",
    [...wsEl("networks").querySelectorAll(".workspace-note")].some((n) => n.textContent.includes("not keyed")),
    "");

// Opening asks the shell for a network and never for a window: which window a
// network belongs in is not the interface's decision (`09` §1.5).
wsCalls.length = 0;
wsEl("networks").querySelector("button").dispatchEvent(new wsWindow.MouseEvent("click", { bubbles: true }));
await settled();
say("clicking a network opens it through the shell",
    wsCalls.includes("open_network"), JSON.stringify(wsCalls));

// Join and create are sheets rather than furniture on the page (`09` §1.2).
say("join and create are not on the page until asked for",
    !wsEl("join-sheet").open && !wsEl("create-sheet").open);
wsEl("join-open").dispatchEvent(new wsWindow.MouseEvent("click", { bubbles: true }));
await settled();
say("the join button opens a sheet", wsEl("join-sheet").open);

// **D29's second designation, and the more common one.** Creating a network
// with a relay another of this member's networks already uses has to warn here
// too — a warning that covered only a network's own relay panel would miss the
// first designation most people ever make (`09` §3).
sharedRelays = [{ relay: RELAY, id: "aa".repeat(32), label: "the workshop" }];
wsAnswering = false;
wsConfirmed.length = 0;
wsCalls.length = 0;
wsEl("create-open").dispatchEvent(new wsWindow.MouseEvent("click", { bubbles: true }));
wsEl("new-name").value = "the other one";
wsEl("new-relay").value = RELAY;
wsEl("maker").dispatchEvent(new wsWindow.Event("submit", { bubbles: true, cancelable: true }));
await settled();
say("creating a network with a shared relay warns first",
    wsConfirmed.length === 1 && !wsCalls.includes("create_network"),
    JSON.stringify(wsConfirmed).slice(0, 60));
say("and the warning names the other network",
    /the workshop/.test(wsConfirmed[0] ?? ""), wsConfirmed[0]);

// It warns rather than refusing: agreeing goes through.
wsAnswering = true;
wsCalls.length = 0;
wsEl("new-name").value = "the other one";
wsEl("new-relay").value = RELAY;
wsEl("maker").dispatchEvent(new wsWindow.Event("submit", { bubbles: true, cancelable: true }));
await settled();
say("and agreeing designates it", wsCalls.includes("create_network"), JSON.stringify(wsCalls));
sharedRelays = [];

// **O21's three landings, which must not collapse into two.** Waiting is a
// success, and *no answer* is not a refusal — an invite is use-limited, so
// telling somebody a join failed is how they spend it on a retry and lock
// themselves out of a network that already holds them.
wsAnswers.join_network = () => ({ admitted: false, identity: "ab".repeat(32), answered: false });
wsEl("join-open").dispatchEvent(new wsWindow.MouseEvent("click", { bubbles: true }));
wsEl("invite").value = "intranet-chat://join/whatever";
wsEl("joiner").dispatchEvent(new wsWindow.Event("submit", { bubbles: true, cancelable: true }));
await settled();
say("a join with no answer is not reported as a refusal",
    /not the same as a refusal/.test(wsEl("workspace-error").textContent),
    wsEl("workspace-error").textContent.slice(0, 60));

wsAnswers.join_network = () => ({ admitted: false, identity: "ab".repeat(32), answered: true });
wsEl("join-open").dispatchEvent(new wsWindow.MouseEvent("click", { bubbles: true }));
wsEl("invite").value = "intranet-chat://join/whatever";
wsEl("joiner").dispatchEvent(new wsWindow.Event("submit", { bubbles: true, cancelable: true }));
await settled();
say("and waiting to be admitted is reported as the success it is",
    /You are in/.test(wsEl("workspace-error").textContent),
    wsEl("workspace-error").textContent.slice(0, 40));

// Leaving says which of the two acts it is about to do (`02` §6.5).
const rowFor = (label) =>
  [...wsEl("networks").querySelectorAll("li")].find((li) => li.textContent.includes(label));
networksAnswer = [
  { id: "aa".repeat(32), label: "the workshop", keyed: true, open: true },
  { id: "bb".repeat(32), label: "book club", keyed: true, open: false },
];
await wsWindow.eval("draw()");
await settled();
say("the open network offers *leave* and a closed one offers *forget*",
    rowFor("the workshop").querySelector(".forget").textContent === "leave" &&
      rowFor("book club").querySelector(".forget").textContent === "forget");

wsAnswering = false;
wsConfirmed.length = 0;
rowFor("book club").querySelector(".forget").dispatchEvent(new wsWindow.MouseEvent("click", { bubbles: true }));
await settled();
say("and forgetting one that is not open says it will not be told",
    /it is not told/.test(wsConfirmed[0] ?? ""), (wsConfirmed[0] ?? "").slice(0, 60));

// ── the conversations group ─────────────────────────────────────────────
//
// `design/09` §1.4 and §1.8: the same window, a second group, because a
// conversation *is* a network (D10) and the member is looking for a name in
// both cases. What separates them is what is possible inside, not where they
// are listed.

console.log("\n── conversations ──");

const AA = "11".repeat(32);
const BB = "22".repeat(32);
conversationsAnswer = [
  { network: "cc".repeat(32), who: AA, label: "mallory", shared: "aa".repeat(32), state: "joined" },
  { network: "", who: BB, label: "dave", shared: "aa".repeat(32), state: "asked" },
  { network: "dd".repeat(32), who: "33".repeat(32), label: "erin", shared: "aa".repeat(32), state: "offered" },
];
await wsWindow.eval("draw()");
await settled();

const convRows = [...wsEl("conversations").querySelectorAll("li")];
say("the workspace lists conversations beside networks",
    convRows.length === 3, `${convRows.length} rows`);

// **Spec 07 §8, in the place it matters most.** A contact list is where
// somebody decides who they are talking to, and the uniqueness key deliberately
// does not fold confusables — so a name never stands alone here.
say("and a name never stands alone: the identity is beside it",
    convRows.every((r) => r.querySelector(".workspace-id")?.textContent.length > 0),
    convRows.map((r) => r.querySelector(".workspace-id")?.textContent).join(","));

// A request that arrived, whether or not the network it came through is open.
const askedRow = convRows.find((r) => r.textContent.includes("dave"));
const answers2 = () => [...askedRow.querySelectorAll(".forget")].map((b) => b.textContent);
say("a request that arrived offers accept and decline",
    answers2().join(",") === "accept,decline", answers2().join(","));

// **Never *waiting for an answer*** (`09` §1.8). A decline sends nothing, so
// this side cannot tell one from somebody who has not looked, and a row that
// claimed to know would invent the difference.
const offered = convRows.find((r) => r.textContent.includes("erin"));
say("one you asked is never reported as waiting for an answer",
    /they will see it when you are both online/i.test(offered.textContent) &&
      !/waiting/i.test(offered.textContent),
    offered.querySelector(".workspace-note").textContent);

// Declining says the thing only this side can know.
wsAnswering = false;
wsConfirmed.length = 0;
wsCalls.length = 0;
askedRow.querySelectorAll(".forget")[1].dispatchEvent(new wsWindow.MouseEvent("click", { bubbles: true }));
await settled();
say("declining warns that the other side is not told",
    /not told|same as you not having looked/i.test(wsConfirmed[0] ?? "") &&
      !wsCalls.includes("decline_conversation"),
    (wsConfirmed[0] ?? "").slice(0, 60));

wsAnswering = true;
wsCalls.length = 0;
askedRow.querySelectorAll(".forget")[1].dispatchEvent(new wsWindow.MouseEvent("click", { bubbles: true }));
await settled();
say("and agreeing declines it locally",
    wsCalls.includes("decline_conversation"), JSON.stringify(wsCalls));

// Accepting joins the conversation's network and opens it — one act to the
// member, and the window is the shell's to make (`09` §1.6).
wsCalls.length = 0;
conversationsAnswer[1].state = "asked";
await wsWindow.eval("draw()");
await settled();
const askedAgain = [...wsEl("conversations").querySelectorAll("li")].find((r) =>
  r.textContent.includes("dave"));
askedAgain.querySelectorAll(".forget")[0].dispatchEvent(new wsWindow.MouseEvent("click", { bubbles: true }));
await settled();
say("accepting joins and then opens the conversation",
    wsCalls.indexOf("accept_conversation") >= 0 &&
      wsCalls.indexOf("open_conversation") > wsCalls.indexOf("accept_conversation"),
    JSON.stringify(wsCalls));

// A row nobody has answered yet cannot be opened: there is no network on this
// side until it is accepted, so a window for it would have nothing to draw.
const pending = [...wsEl("conversations").querySelectorAll("li")].find((r) =>
  r.textContent.includes("erin"));
say("a conversation not yet joined cannot be opened",
    pending.querySelector(".workspace-open").disabled === true);

wsCalls.length = 0;
[...wsEl("conversations").querySelectorAll("li")]
  .find((r) => r.textContent.includes("mallory"))
  .querySelector(".workspace-open")
  .dispatchEvent(new wsWindow.MouseEvent("click", { bubbles: true }));
await settled();
say("and one that is joined opens through the shell",
    wsCalls.includes("open_conversation"), JSON.stringify(wsCalls));

// The empty case says what to do rather than nothing at all.
conversationsAnswer = [];
await wsWindow.eval("draw()");
await settled();
say("with none, the group says so instead of sitting empty",
    !wsEl("conversations-empty").hidden);

// **Drawn once and then stale is a defect this window actually shipped with.**
// A network founded here said *not keyed in yet* for as long as the window
// stayed open — the epoch key is written a beat after the node starts, the row
// had already been drawn, and nothing asked again. A request arriving was the
// same failure with worse consequences: §1.8 puts it in this list, and it
// appeared on whatever draw happened next.
conversationsAnswer = [
  { network: "", who: BB, label: "dave", shared: "aa".repeat(32), state: "asked" },
];
await wsListeners["kols://conversations"]({ payload: ["aa".repeat(32)] });
await settled();
say("a request arriving redraws the group without anybody asking",
    wsEl("conversations").querySelectorAll("li").length === 1,
    String(wsEl("conversations").querySelectorAll("li").length));

networksAnswer = [
  { id: "aa".repeat(32), label: "the workshop", keyed: false, open: true },
];
await wsWindow.eval("drawNetworks()");
await settled();
say("a network not yet keyed says so", /not keyed/.test(wsEl("networks").textContent));
networksAnswer = [{ id: "aa".repeat(32), label: "the workshop", keyed: true, open: true }];
await wsListeners["kols://keys"]({ payload: ["aa".repeat(32)] });
await settled();
say("and stops saying it once the key arrives, without a redraw being asked for",
    !/not keyed/.test(wsEl("networks").textContent),
    wsEl("networks").textContent.trim().slice(0, 40));

// ── the gate in front of everything ────────────────────────────────────
//
// `design/02` §6.3: a node runs only once somebody has logged in, because
// starting one first would need the seeds unwrappable without the password — at
// which point the password protects nothing at rest.

console.log("\n── the lock ──");

const shown = () =>
  ["lock", "workspace", "settings"].filter((id) => !wsEl(id).hidden);

// A first run: no account. The account is forced rather than offered, so there
// is no way past this screen that does not make one.
wsAnswers.account_state = () => ({ exists: false, unlocked: false, username: null, unprotected: 2 });
await wsWindow.eval("gate()");
await settled();
say("with no account, the first run is what shows", JSON.stringify(shown()) === '["lock"]', JSON.stringify(shown()));
say("and it is the first-run form, not a login",
    !wsEl("first-run").hidden && wsEl("login").hidden);
// Said before the password is chosen rather than after it is lost.
const warned = wsEl("lock").querySelector('[data-kols="no-recovery"]');
say("it says there is no reset before asking for one",
    warned !== null && warned.textContent.includes("no one can recover it"),
    warned ? warned.textContent.trim().slice(0, 40) : "(absent)");
// And it says what it is about to protect, rather than asking for nothing given.
say("and names what is currently unprotected",
    wsEl("first-run-note").textContent.includes("2 networks"),
    wsEl("first-run-note").textContent.slice(0, 50));

// A password typed twice and not matching must not reach the shell: there is no
// reset behind it, so a mistype here is every identity on the disk.
wsCalls.length = 0;
wsEl("first-run-name").value = "corey";
wsEl("first-run-password").value = "one";
wsEl("first-run-again").value = "another";
wsEl("first-run").dispatchEvent(new wsWindow.Event("submit", { bubbles: true, cancelable: true }));
await settled();
say("two passwords that differ never reach the shell",
    !wsCalls.includes("create_account") && !wsEl("first-run-error").hidden,
    wsEl("first-run-error").textContent);

// An existing account: a login, greeting whoever it belongs to. A username is
// not a secret, and a login that cannot say whose it is makes a shared machine
// guesswork.
wsAnswers.account_state = () => ({ exists: true, unlocked: false, username: "corey", unprotected: 0 });
await wsWindow.eval("gate()");
await settled();
say("with an account, it is a login", !wsEl("login").hidden && wsEl("first-run").hidden);
say("and it greets whoever it belongs to",
    wsEl("login-greeting").textContent.includes("corey"), wsEl("login-greeting").textContent);

// A wrong password reports and stays put rather than falling through.
wsAnswers.unlock = () => {
  throw new Error("that password does not unlock this installation");
};
wsEl("login-password").value = "wrong";
wsEl("login").dispatchEvent(new wsWindow.Event("submit", { bubbles: true, cancelable: true }));
await settled();
say("a wrong password says so and stays on the lock screen",
    !wsEl("login-error").hidden && JSON.stringify(shown()) === '["lock"]',
    wsEl("login-error").textContent);

// And the right one gets in.
wsAnswers.unlock = () => 0;
wsAnswers.account_state = () => ({ exists: true, unlocked: true, username: "corey", unprotected: 0 });
wsEl("login-password").value = "right";
wsEl("login").dispatchEvent(new wsWindow.Event("submit", { bubbles: true, cancelable: true }));
await settled();
say("the right one opens the window", JSON.stringify(shown()) === '["workspace"]', JSON.stringify(shown()));
say("and the password is not left in the field", wsEl("login-password").value === "");

// Locking hides the window and deliberately does not stop the node.
wsCalls.length = 0;
wsAnswers.account_state = () => ({ exists: true, unlocked: false, username: "corey", unprotected: 0 });
wsEl("lock-now").dispatchEvent(new wsWindow.MouseEvent("click", { bubbles: true }));
await settled();
say("locking returns to the lock screen", JSON.stringify(shown()) === '["lock"]', JSON.stringify(shown()));
say("and it locks rather than stopping the node",
    wsCalls.includes("lock") && !wsCalls.includes("stop_node"),
    JSON.stringify(calls));

// ── the copy you can move ──────────────────────────────────────────────
//
// Portability, not recovery. The seed is the member and lives on one disk, so
// this is the only thing that survives the disk.

console.log("\n── the export ──");

// Offered right after the account is made, and not in the way — a first run that
// refused to proceed without a file saved somewhere is a flow people defeat.
wsAnswers.account_state = () => ({ exists: false, unlocked: false, username: null, unprotected: 1 });
await wsWindow.eval("gate()");
await settled();
wsAnswers.account_state = () => ({ exists: true, unlocked: true, username: "corey", unprotected: 0 });
wsEl("first-run-name").value = "corey";
wsEl("first-run-password").value = "same";
wsEl("first-run-again").value = "same";
wsEl("first-run").dispatchEvent(new wsWindow.Event("submit", { bubbles: true, cancelable: true }));
await settled();
say("making an account offers the copy straight after",
    !wsEl("settings").hidden && !wsEl("export-nudge").hidden,
    `settings ${wsEl("settings").hidden ? "hidden" : "shown"}, nudge ${wsEl("export-nudge").hidden ? "hidden" : "shown"}`);
say("and it did not block on it — the account was made",
    wsCalls.includes("create_account"));

// The passphrase must not be the login password, and the window says so where
// somebody is choosing one.
const apart = wsEl("settings").textContent;
say("the file's passphrase is said not to be the login password",
    apart.includes("Not your login password"),
    apart.includes("Not your login password") ? "said" : "(absent)");

// An empty passphrase never reaches the shell: this file is every identity.
wsCalls.length = 0;
wsAnswers.export_bundle = (args) => {
  if (!args.passphrase) throw new Error("a passphrase is required — this file is every identity here");
  return 2;
};
wsEl("export-path").value = "/tmp/backup";
wsEl("export-pass").value = "";
wsEl("export-form").dispatchEvent(new wsWindow.Event("submit", { bubbles: true, cancelable: true }));
await settled();
say("an empty passphrase is refused and said so",
    !wsEl("export-error").hidden, wsEl("export-error").textContent.slice(0, 40));

wsEl("export-pass").value = "paper passphrase";
wsEl("export-form").dispatchEvent(new wsWindow.Event("submit", { bubbles: true, cancelable: true }));
await settled();
say("a written copy reports how many networks are in it",
    !wsEl("export-done").hidden && wsEl("export-done").textContent.includes("2 networks"),
    wsEl("export-done").textContent.slice(0, 50));
// It is the only thing between whoever picks the file up and every identity in it.
say("and the passphrase is not left in the field", wsEl("export-pass").value === "");

// A restore says all three things. One that reported only what it added would be
// silent about the network it deliberately left alone.
wsAnswers.import_bundle = () => ({
  added: ["the workshop"],
  skipped: ["already here"],
  refused: [],
});
wsEl("import-path").value = "/tmp/backup";
wsEl("import-pass").value = "paper passphrase";
wsEl("import-form").dispatchEvent(new wsWindow.Event("submit", { bubbles: true, cancelable: true }));
await settled();
const restored = wsEl("import-done").textContent;
say("a restore says what it took and what it left alone",
    restored.includes("the workshop") && restored.includes("already here"),
    restored);
say("and its passphrase is not left either", wsEl("import-pass").value === "");

// ── the ceiling that stops a disk filling up ───────────────────────────
await wsWindow.eval('drawCeiling()');
say("the ceiling is shown in whole gigabytes", wsEl("ceiling-gb").value === "2",
    wsEl("ceiling-gb").value);
say("usage is reported against it, with the network count",
    wsEl("ceiling-usage").textContent.includes("1.20 GB of 2 GB") &&
      wsEl("ceiling-usage").textContent.includes("across 3 networks"),
    wsEl("ceiling-usage").textContent);

// A ceiling of nothing would stop the application keeping what somebody is
// reading, which is not a contribution setting and must not behave like one.
wsEl("ceiling-gb").value = "0";
wsEl("ceiling-form").dispatchEvent(
  new wsWindow.Event("submit", { bubbles: true, cancelable: true }),
);
await settled();
say("a ceiling is floored at one gigabyte, never zero",
    ceiling.ceiling === 1024 * 1024 * 1024, String(ceiling.ceiling));

// ── the conversation window ────────────────────────────────────────────
//
// A third document (`design/09` §1.6, D40). One person and one implied channel,
// which is the whole of what a `conversation`-profile network has (spec 07
// §1.2) — so what is checked here is as much what it does *not* draw as what it
// does. Its own DOM, and its own URL, because the network it is for is carried
// in the address rather than asked for: several are open at once and the shell's
// single "open network" is not an answer here.

console.log("\n══ a conversation window ══");

const CONV = "cc".repeat(32);
const cvHtml = fs
  .readFileSync(`${UI}/conversation.html`, "utf8")
  .replace(/<script src="conversation.js"><\/script>/, "");
const cvSource = fs.readFileSync(`${UI}/conversation.js`, "utf8");

const cvDom = new JSDOM(cvHtml, {
  runScripts: "outside-only",
  pretendToBeVisual: true,
  url: `http://localhost/conversation.html?network=${CONV}`,
});
const cvWindow = cvDom.window;

let who = { who: "11".repeat(32), label: "mallory", shared: "aa".repeat(32), state: "joined" };
let saidHere = [
  { id: "m1", body: "are you there", at: "10:01", mine: false, withdrawn: false },
  { id: "m2", body: "here", at: "10:02", mine: true, withdrawn: false },
];
const cvCalls = [];
const cvSent = [];
const cvAnswers = {
  conversation_who: () => who,
  conversation_read: () => ({ messages: saidHere }),
  conversation_send: (args) => {
    cvSent.push(args);
    return null;
  },
};
const cvListeners = {};
cvWindow.__TAURI__ = {
  core: {
    invoke: async (name, args) => {
      cvCalls.push([name, args]);
      const answer = cvAnswers[name];
      if (!answer) throw new Error(`no stub for ${name}`);
      return answer(args ?? {});
    },
  },
  event: {
    listen: async (name, run) => {
      cvListeners[name] = run;
      return () => {};
    },
  },
  window: { getCurrentWindow: () => ({ setTitle: async () => {} }) },
};
cvWindow.addEventListener("error", (e) => problems.push(`conversation error: ${e.error?.stack ?? e.message}`));
cvWindow.addEventListener("unhandledrejection", (e) => problems.push(`conversation rejected: ${e.reason?.stack ?? e.reason}`));
cvWindow.eval(cvSource);

const cvEl = (id) => cvWindow.document.getElementById(id);
await settled();

// Which conversation this window is for came from the address, so the shell can
// open several and each knows its own.
say("the window reads which conversation it is from its address",
    cvCalls.every(([, args]) => !args || args.network === CONV),
    JSON.stringify(cvCalls.map(([n]) => n)));

say("it draws who it is with", cvEl("who").textContent === "mallory", cvEl("who").textContent);

// **Spec 07 §8 again, and this is the window where it bites**: somebody is
// deciding who they are talking to, and the uniqueness key does not fold
// confusables. The network they were met in is there too, because a request
// binds exactly that pair (§6.2).
say("with the identity beside the name, and where they were met",
    /^11111111 · met in aaaaaaaa/.test(cvEl("where").textContent),
    cvEl("where").textContent);

// What it deliberately has not got. A `conversation`-profile network has one
// implied channel and no roles, so a channel rail or a roster would be
// furniture that is always empty and controls that cannot exist.
// Asked of the document rather than of its source, because the source says
// these are absent on purpose and a text search would find the sentence saying
// so.
say("and no channel rail, no roster and no settings",
    ["channel-list", "roster-list", "open-settings", "invite", "settings"].every(
      (id) => cvWindow.document.getElementById(id) === null,
    ) && cvWindow.document.querySelectorAll('[data-kols]').length > 0);

say("the messages are drawn", cvEl("messages").querySelectorAll(".said").length === 2,
    String(cvEl("messages").querySelectorAll(".said").length));
say("and this member's own are marked apart",
    cvEl("messages").querySelectorAll(".said.mine").length === 1);

// **Merge by id, never append** — the invariant this whole client is built on.
// A record arriving over gossip is also inside the segment that follows it, so
// a window that appended what it was handed would show every message twice.
// Delivered here as the same two messages arriving again.
cvListeners["kols://records"]({ payload: [CONV] });
await settled();
say("a redelivery of the same messages does not double them",
    cvEl("messages").querySelectorAll(".said").length === 2,
    String(cvEl("messages").querySelectorAll(".said").length));

// An event from another network must not redraw this one: several nodes run at
// once (`09` §2).
saidHere = [...saidHere, { id: "m3", body: "from elsewhere", at: "10:03", mine: false, withdrawn: false }];
cvListeners["kols://records"]({ payload: ["aa".repeat(32)] });
await settled();
say("and something that arrived in another network does not redraw this one",
    cvEl("messages").querySelectorAll(".said").length === 2,
    String(cvEl("messages").querySelectorAll(".said").length));
cvListeners["kols://records"]({ payload: [CONV] });
await settled();
say("while something that arrived here does",
    cvEl("messages").querySelectorAll(".said").length === 3);

// **Withdrawn is not deleted and must not read as if it were** (`01` §6): it
// stops conformant clients drawing a message and retracts nothing anybody
// already fetched.
saidHere = [{ id: "m1", body: "", at: "10:01", mine: false, withdrawn: true }];
cvListeners["kols://records"]({ payload: [CONV] });
await settled();
const gone = cvEl("messages").querySelector(".said-body");
say("a withdrawn message reads as withdrawn rather than as erased",
    gone.classList.contains("withdrawn") && gone.textContent === "withdrawn",
    gone.textContent);

// Sending goes through the shell, clears the field, and never posts nothing.
cvSent.length = 0;
cvEl("body").value = "   ";
cvEl("composer").dispatchEvent(new cvWindow.Event("submit", { bubbles: true, cancelable: true }));
await settled();
say("an empty line is never sent", cvSent.length === 0);

cvEl("body").value = "hello";
cvEl("composer").dispatchEvent(new cvWindow.Event("submit", { bubbles: true, cancelable: true }));
await settled();
say("and a line is sent through the shell with the conversation it is in",
    cvSent.length === 1 && cvSent[0].body === "hello" && cvSent[0].network === CONV,
    JSON.stringify(cvSent));
say("the field is cleared after it goes", cvEl("body").value === "");

// ── D39's limit, said where it is true — §1.9 ──────────────────────────
//
// A conversation borrows its rendezvous from the network it was arranged in, on
// a permission recomputed every time and never stored. When the shared
// membership ends, nothing new can cross — and that renders identically to
// nobody talking, which is the one thing this window must not let it look like.
who = { ...who, state: "adrift" };
await cvWindow.eval("drawWho()");
await settled();
say("a conversation with nowhere left to meet says so rather than going quiet",
    !cvEl("stopped").hidden && /no longer share a network/.test(cvEl("stopped").textContent),
    cvEl("stopped").textContent.slice(0, 50));
say("and it says what is still readable, because nothing was lost",
    /stays readable/.test(cvEl("stopped").textContent));
// Not drawn as a failure: the loan ended when the shared membership did, which
// is what borrowing rather than designating means.
say("it is not reported as an error",
    cvEl("error").hidden && !/error|failed|broken/i.test(cvEl("stopped").textContent));

who = { ...who, state: "joined" };
await cvWindow.eval("drawWho()");
await settled();
say("and the notice goes when it is no longer true", cvEl("stopped").hidden);

// A window the shell opened without one says so, rather than sitting blank and
// looking like a conversation with nothing in it.
const strayDom = new JSDOM(cvHtml, { runScripts: "outside-only", url: "http://localhost/conversation.html" });
strayDom.window.__TAURI__ = cvWindow.__TAURI__;
strayDom.window.eval(cvSource);
await settled();
say("a window opened without a conversation says so",
    !strayDom.window.document.getElementById("error").hidden,
    strayDom.window.document.getElementById("error").textContent);

console.log(problems.length ? "\nPROBLEMS:\n" + problems.join("\n") : "\nno uncaught errors");
process.exit(0);
