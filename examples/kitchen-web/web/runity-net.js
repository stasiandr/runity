// Rooms for the kitchen in the browser: what `src/lobby.rs` and the
// engine's `net::page` wire call as `runityNet.*`.
//
// The host opens a room: a PeerJS peer named after a short code, reached
// through PeerJS's public signalling broker (the page is static, it has no
// server of its own). A guest connects to that name; from then on the two
// browsers talk over a WebRTC data channel directly, and the broker is out
// of it. Datagrams start with the sender's id (little-endian u32), so the
// host learns which channel is whose from what comes in.

const PREFIX = "runity-kitchen-rush-";
// No 0/O, 1/I/L: a code is read out loud and typed on a phone.
const LETTERS = "ABCDEFGHJKMNPQRSTUVWXYZ23456789";
const JOIN_TIMEOUT_MS = 20000;
// How two browsers find a way to each other: STUN for the address a home
// router shows the world (most pairs connect on that), a TURN relay for
// the rest (two phones on mobile data) — Metered's, below, when the site
// was built with one; PeerJS's own is the fallback.
const PEER_OPTIONS = {
  debug: 1,
  config: {
    iceServers: [
      { urls: ["stun:stun.l.google.com:19302", "stun:stun.cloudflare.com:3478"] },
      { urls: ["turn:eu-0.turn.peerjs.com:3478", "turn:us-0.turn.peerjs.com:3478"], username: "peerjs", credential: "peerjsp" },
    ],
  },
};

// Where fresh TURN credentials come from (a Metered app's REST endpoint),
// written in by web/build.sh from RUNITY_TURN_URL; left as is, there is no
// relay and only devices STUN can join meet.
const TURN_URL = "__RUNITY_TURN_URL__";

// The ICE servers for a new peer: the relay's credentials fetched fresh,
// or STUN alone when there is no relay or it does not answer in time.
async function peerOptions() {
  if (!TURN_URL || TURN_URL.startsWith("__")) return PEER_OPTIONS;
  try {
    const answer = await Promise.race([
      fetch(TURN_URL).then((r) => (r.ok ? r.json() : null)),
      new Promise((r) => setTimeout(() => r(null), 4000)),
    ]);
    if (Array.isArray(answer) && answer.length) {
      const config = { iceServers: [...PEER_OPTIONS.config.iceServers.slice(0, 1), ...answer] };
      // `?relay`: through the relay only, as two phones behind strict NATs
      // would go — to check it works.
      if (new URLSearchParams(location.search).has("relay")) config.iceTransportPolicy = "relay";
      return { ...PEER_OPTIONS, config };
    }
  } catch (_) {}
  return PEER_OPTIONS;
}

function randomCode(n = 5) {
  const bytes = crypto.getRandomValues(new Uint8Array(n));
  return Array.from(bytes, (b) => LETTERS[b % LETTERS.length]).join("");
}

function nameOf() {
  let name = null;
  try {
    name = localStorage.getItem("runity:name");
  } catch (_) {}
  if (!name) {
    name = "Cook " + Math.floor(100 + Math.random() * 900);
    try {
      localStorage.setItem("runity:name", name);
    } catch (_) {}
  }
  return name;
}

const net = {
  wasm: null,
  _state: "",
  _peer: null,
  _code: null,
  // Host: each guest's channel by the id it sends from.
  _byId: new Map(),
  // Guest: the channel to the host.
  _host: null,
  _timer: null,
  onRoom: () => {},

  state() {
    return this._state;
  },

  name() {
    return nameOf();
  },

  code() {
    return this._code;
  },

  link() {
    if (!this._code) return location.href;
    const url = new URL(location.href);
    url.hash = "join=" + this._code;
    return url.toString();
  },

  _deliver(data) {
    if (!this.wasm) return;
    const bytes = data instanceof ArrayBuffer ? new Uint8Array(data) : new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
    this.wasm.runity_net_receive(bytes);
    return bytes;
  },

  _fail(why) {
    this._state = "failed:" + why;
    this.onRoom(null);
    this._say("Could not join: " + why, 9000);
  },

  // A line at the bottom of the page: how joining goes, step by step.
  _say(text, ms) {
    window.runityToast && window.runityToast(text, ms);
  },

  host() {
    this.leave();
    const code = randomCode();
    this._code = code;
    this._state = "hosting";
    this._everOpened = false;
    this._open(code, 0);
    return code;
  },

  // The room's peer at the broker, by the room's code. The code is kept for
  // as long as the room is: a phone that locked or switched apps comes back
  // to the same code, so the link a friend has still works.
  async _open(code, tries) {
    const options = await peerOptions();
    if (this._state !== "hosting" || this._code !== code) return;
    const peer = new Peer(PREFIX + code, options);
    this._peer = peer;
    let opened = false;
    peer.on("open", () => {
      opened = true;
      this.onRoom(code);
      if (tries === 0) this._say("Room " + code + " is open. Keep this page in front while friends join.", 6000);
    });
    peer.on("connection", (conn) => {
      conn.on("data", (data) => {
        const bytes = this._deliver(data);
        if (bytes && bytes.length >= 4) {
          const id = new DataView(bytes.buffer, bytes.byteOffset, 4).getUint32(0, true);
          if (this._byId.get(id) !== conn) this._byId.set(id, conn);
        }
      });
      conn.on("close", () => {
        for (const [id, c] of this._byId) if (c === conn) this._byId.delete(id);
      });
    });
    peer.on("disconnected", () => {
      // The broker dropped us; channels already open carry on. Come back so
      // new guests can still find the room.
      if (this._peer === peer && !peer.destroyed) peer.reconnect();
    });
    peer.on("error", (e) => {
      if (this._peer !== peer || this._state !== "hosting") return;
      if (e.type === "unavailable-id" && !opened && tries === 0 && !this._everOpened) {
        // Someone else has this code: another one.
        this.leave();
        this.host();
      } else if (e.type === "unavailable-id" || e.type === "network" || e.type === "server-error" || e.type === "socket-error" || e.type === "socket-closed") {
        // Our own room, still held at the broker from before the phone
        // slept, or the broker out of reach: the same code again shortly.
        try { peer.destroy(); } catch (_) {}
        setTimeout(() => {
          if (this._state === "hosting" && this._code === code) this._open(code, tries + 1);
        }, Math.min(1000 * (tries + 1), 5000));
      } else if (e.type !== "peer-unavailable") {
        console.warn("room:", e);
      }
    });
    peer.on("open", () => { this._everOpened = true; });
  },

  // Back in front after the phone slept or another app was up: the room
  // is found at the broker again.
  _wake() {
    const peer = this._peer;
    if (this._state !== "hosting" || !peer) return;
    if (peer.destroyed) this._open(this._code, 1);
    else if (peer.disconnected) peer.reconnect();
  },

  async join(code) {
    code = String(code || "").trim().toUpperCase().replace(/[^A-Z0-9]/g, "");
    if (!code) return;
    this.leave();
    this._code = code;
    this._state = "joining";
    this._say("Joining " + code + "…", JOIN_TIMEOUT_MS);
    const options = await peerOptions();
    if (this._state !== "joining" || this._code !== code) return;
    const peer = new Peer(options);
    this._peer = peer;
    let step = "reaching the broker";
    this._say("Joining " + code + ": " + step + "…", JOIN_TIMEOUT_MS);
    this._timer = setTimeout(() => {
      if (this._state === "joining") {
        this._fail("no answer from kitchen " + code + " (stuck " + step + ")");
        this._close();
      }
    }, JOIN_TIMEOUT_MS);
    peer.on("open", () => {
      step = "calling the kitchen";
      this._say("Joining " + code + ": " + step + "…", JOIN_TIMEOUT_MS);
      const conn = peer.connect(PREFIX + code, { reliable: false, serialization: "raw" });
      this._host = conn;
      // How the two phones find a way to each other, for the message if
      // they do not.
      const watch = () => {
        const pc = conn.peerConnection;
        if (!pc) return setTimeout(watch, 200);
        pc.addEventListener("iceconnectionstatechange", () => {
          const ice = pc.iceConnectionState;
          if (ice === "checking") {
            step = "finding a way between the phones";
            this._say("Joining " + code + ": " + step + "…", JOIN_TIMEOUT_MS);
          }
          if (ice === "failed" && this._state === "joining") {
            clearTimeout(this._timer);
            this._fail("the two devices could not reach each other — their networks block it. Try both on the same Wi-Fi.");
            this._close();
          }
        });
      };
      watch();
      conn.on("open", () => {
        clearTimeout(this._timer);
        this._state = "open";
        this._say("In kitchen " + code + "!", 2500);
      });
      conn.on("data", (data) => this._deliver(data));
      conn.on("close", () => {
        if (this._host === conn) this._host = null;
      });
    });
    peer.on("error", (e) => {
      if (this._state !== "joining") return;
      clearTimeout(this._timer);
      this._fail(e.type === "peer-unavailable" ? "no kitchen with the code " + code : String(e.message || e.type));
      this._close();
    });
  },

  // Ask for a friend's code: the page's dialog, which calls join.
  ask() {
    const dialog = document.getElementById("join-dialog");
    if (!dialog) {
      const code = prompt("Room code");
      if (code) this.join(code);
      return;
    }
    dialog.hidden = false;
    const input = dialog.querySelector("input");
    input.value = "";
    setTimeout(() => input.focus(), 50);
  },

  async invite() {
    const url = this.link();
    const text = "Cook with me in Kitchen Rush — room " + this._code;
    try {
      if (navigator.share) {
        await navigator.share({ title: "Kitchen Rush", text, url });
        return;
      }
    } catch (_) {}
    try {
      await navigator.clipboard.writeText(url);
      window.runityToast && window.runityToast("Link copied: " + url);
    } catch (_) {
      prompt("Send this link to a friend", url);
    }
  },

  send(to, bytes) {
    // The view is into wasm's memory, which may move: copied out first.
    const copy = bytes.slice();
    const conn = this._state === "open" ? this._host : this._byId.get(to);
    if (conn && conn.open) {
      try {
        conn.send(copy.buffer);
      } catch (_) {}
    }
  },

  _close() {
    if (this._peer) {
      try {
        this._peer.destroy();
      } catch (_) {}
    }
    this._peer = null;
    this._host = null;
    this._byId.clear();
  },

  leave() {
    clearTimeout(this._timer);
    this._close();
    this._code = null;
    this._state = "";
    this.onRoom(null);
  },
};

window.runityNet = net;
document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "visible") net._wake();
});
addEventListener("pageshow", () => net._wake());
