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
  set_contribution: (args) => {
    saved.push(args);
    return null;
  },
  // What another of this member's networks already designates — D29. The
  // workspace answers this for real; here it is whatever the check under test
  // needs it to be.
  shared_relays: () => sharedRelays,
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
await listeners["kols://records"]({ payload: ["c1", true] });
await settled();
say("a message landing mid-timeline is marked", JSON.stringify(rows()) === "[false,true,false]", JSON.stringify(rows()));
await window.eval("refresh()");
await settled();
say("the mark survives a redraw", JSON.stringify(rows()) === "[false,true,false]", JSON.stringify(rows()));

// A redraw under a reader who has not moved must keep it; arriving again clears it.
await listeners["kols://records"]({ payload: ["c1", true] });
await settled();
say("another arrival keeps the earlier mark", JSON.stringify(rows()) === "[false,true,false]", JSON.stringify(rows()));
el("channel-list").querySelector("button").dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
await settled();
say("clicking the channel clears them", rows().every((f) => !f), JSON.stringify(rows()));

// ── what a mark is not for ─────────────────────────────────────────────
messages = [
  ...messages,
  { id: "m4", author: "corey", author_id: "id-corey-0001", at: "10:02", at_millis: 3000, body: "mine",
    edited: false, withdrawn: false, redacted: false, pinned: false, reactions: [], mine: true },
  { id: "m5", author: "sam", author_id: "id-sam-0002", at: "10:03", at_millis: 4000, body: "theirs",
    edited: false, withdrawn: false, redacted: false, pinned: false, reactions: [], mine: false },
];
await listeners["kols://records"]({ payload: ["c1", true] });
await settled();
say("your own message is never marked", JSON.stringify(rows()) === "[false,false,false,false,true]",
    JSON.stringify(rows()));

// Hovering it is reading it.
const marked = [...el("messages").querySelectorAll(".message")].find((r) => r.classList.contains("fresh"));
marked?.dispatchEvent(new window.MouseEvent("mouseenter", { bubbles: false }));
await settled();
say("hovering a marked message clears it", rows().every((f) => !f), JSON.stringify(rows()));

// ── being told from outside the window ─────────────────────────────────
await listeners["kols://records"]({ payload: ["c2", true] });
await settled();
say("unread reaches the title", titles.at(-1) === "ko-ls (1)", titles.at(-1));

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
await listeners["kols://records"]({ payload: ["c1", true] });
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

// ── the ceiling that stops a disk filling up ───────────────────────────
await window.drawCeiling();
say("the ceiling is shown in whole gigabytes", el("ceiling-gb").value === "2",
    el("ceiling-gb").value);
say("usage is reported against it, with the network count",
    el("ceiling-usage").textContent.includes("1.20 GB of 2 GB") &&
      el("ceiling-usage").textContent.includes("across 3 networks"),
    el("ceiling-usage").textContent);

// A ceiling of nothing would stop the application keeping what somebody is
// reading, which is not a contribution setting and must not behave like one.
el("ceiling-gb").value = "0";
el("ceiling-form").dispatchEvent(
  new window.Event("submit", { bubbles: true, cancelable: true }),
);
await settled();
say("a ceiling is floored at one gigabyte, never zero",
    ceiling.ceiling === 1024 * 1024 * 1024, String(ceiling.ceiling));

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
// most people make. A warning that covered only the panel would miss it.
answering = false;
confirmed.length = 0;
calls.length = 0;
el("new-name").value = "the other one";
el("new-relay").value = RELAY;
el("maker").dispatchEvent(new window.Event("submit", { bubbles: true, cancelable: true }));
await settled();
say("creating a network with a shared relay warns as well",
    confirmed.length === 1 && !calls.includes("create_network"));

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

console.log(problems.length ? "\nPROBLEMS:\n" + problems.join("\n") : "\nno uncaught errors");
process.exit(0);
