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
// router shows the world (most pairs connect on that), PeerJS's own TURN
// relay as the last resort.
const PEER_OPTIONS = {
  debug: 1,
  config: {
    iceServers: [
      { urls: ["stun:stun.l.google.com:19302", "stun:stun.cloudflare.com:3478"] },
      { urls: ["turn:eu-0.turn.peerjs.com:3478", "turn:us-0.turn.peerjs.com:3478"], username: "peerjs", credential: "peerjsp" },
    ],
  },
};

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
  },

  host() {
    this.leave();
    const code = randomCode();
    this._code = code;
    this._state = "hosting";
    const peer = new Peer(PREFIX + code, PEER_OPTIONS);
    this._peer = peer;
    peer.on("open", () => this.onRoom(code));
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
      if (e.type === "unavailable-id") {
        // Someone has this code already: another one.
        this.host();
        this.onRoom(this._code);
      } else if (e.type !== "peer-unavailable") {
        console.warn("room:", e);
      }
    });
    return code;
  },

  join(code) {
    code = String(code || "").trim().toUpperCase().replace(/[^A-Z0-9]/g, "");
    if (!code) return;
    this.leave();
    this._code = code;
    this._state = "joining";
    const peer = new Peer(PEER_OPTIONS);
    this._peer = peer;
    this._timer = setTimeout(() => {
      if (this._state === "joining") {
        this._fail("no kitchen answered at " + code);
        this._close();
      }
    }, JOIN_TIMEOUT_MS);
    peer.on("open", () => {
      const conn = peer.connect(PREFIX + code, { reliable: false, serialization: "raw" });
      this._host = conn;
      conn.on("open", () => {
        clearTimeout(this._timer);
        this._state = "open";
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
