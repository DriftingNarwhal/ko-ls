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
  { id: "m1", author: "corey", author_id: "id-corey-0001", at: "10:00", body: "one",
    edited: false, withdrawn: false, redacted: false, pinned: false, reactions: [], mine: true },
  { id: "m2", author: "sam", author_id: "id-sam-0002", at: "10:01", body: "two",
    edited: false, withdrawn: false, redacted: false, pinned: false, reactions: [], mine: false },
];
let waiting = [];
let people = [
  { identity: "id-corey-0001", short: "id-cor", name: "corey", connected: false, you: true },
  { identity: "id-sam-0002", short: "id-sam", name: "sam", connected: false, you: false },
];

const calls = [];
const answers = {
  me: () => me,
  sidebar: () => channels.map((channel) => ({ kind: "channel", channel })),
  open_channel: ({ channel }) => ({ channel, messages, authors: 2, refused: [] }),
  people: () => people,
  waiting: () => waiting,
  relays: () => ({ designated: [], live: [], cached: [], standing: "none", detail: "" }),
  networks: () => [],
  reorg: () => null,
  settings: () => [],
  roles: () => [],
};

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
const rows = () => [...el("messages").children].map((r) => r.classList.contains("fresh"));
say("first visit highlights nothing", rows().every((f) => !f), JSON.stringify(rows()));
messages = [
  messages[0],
  { id: "m3", author: "sam", author_id: "id-sam-0002", at: "10:00:30", body: "late",
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
  { id: "m4", author: "corey", author_id: "id-corey-0001", at: "10:02", body: "mine",
    edited: false, withdrawn: false, redacted: false, pinned: false, reactions: [], mine: true },
  { id: "m5", author: "sam", author_id: "id-sam-0002", at: "10:03", body: "theirs",
    edited: false, withdrawn: false, redacted: false, pinned: false, reactions: [], mine: false },
];
await listeners["kols://records"]({ payload: ["c1", true] });
await settled();
say("your own message is never marked", JSON.stringify(rows()) === "[false,false,false,false,true]",
    JSON.stringify(rows()));

// Hovering it is reading it.
const marked = [...el("messages").children].find((r) => r.classList.contains("fresh"));
marked?.dispatchEvent(new window.MouseEvent("mouseenter", { bubbles: false }));
await settled();
say("hovering a marked message clears it", rows().every((f) => !f), JSON.stringify(rows()));

// ── being told from outside the window ─────────────────────────────────
await listeners["kols://records"]({ payload: ["c2", true] });
await settled();
say("unread reaches the title", titles.at(-1) === "ko-ls (1)", titles.at(-1));

console.log(problems.length ? "\nPROBLEMS:\n" + problems.join("\n") : "\nno uncaught errors");
process.exit(0);
