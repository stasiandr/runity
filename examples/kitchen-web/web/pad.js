// Sticks and buttons on the screen, for a phone: the left thumb walks,
// the right one grabs, chops, throws and points. They come into the game
// as a pad's (`runity_pad_axis`, `runity_pad_button`), so `input.ron`'s
// `LeftX`/`LeftY` and `Pad(South)` read them like a real pad's.
(() => {
  const touch = matchMedia("(pointer: coarse)").matches || navigator.maxTouchPoints > 0;
  document.body.classList.toggle("touch", touch);
  const wasm = () => window.runityNet && window.runityNet.wasm;

  const stick = document.getElementById("stick");
  const knob = document.getElementById("knob");
  const zone = document.getElementById("stick-zone");
  const RADIUS = 56;
  const DEAD = 0.12;
  let finger = null;
  let home = null;

  function axes(x, y) {
    const w = wasm();
    if (!w) return;
    w.runity_pad_axis("LeftX", x);
    w.runity_pad_axis("LeftY", y);
  }

  function place(dx, dy) {
    const len = Math.hypot(dx, dy);
    const k = len > RADIUS ? RADIUS / len : 1;
    dx *= k;
    dy *= k;
    knob.style.transform = `translate(${dx}px, ${dy}px)`;
    let x = dx / RADIUS;
    let y = -dy / RADIUS;
    const m = Math.hypot(x, y);
    if (m < DEAD) {
      x = 0;
      y = 0;
    } else {
      // Past the dead zone the stick starts from nothing, not from DEAD.
      const s = (m - DEAD) / (1 - DEAD) / m;
      x *= s;
      y *= s;
    }
    axes(x, y);
  }

  function release() {
    finger = null;
    stick.classList.remove("held");
    stick.style.left = "";
    stick.style.top = "";
    knob.style.transform = "";
    axes(0, 0);
  }

  // The stick comes to where the thumb lands, anywhere in the left part.
  zone.addEventListener("pointerdown", (e) => {
    if (finger !== null) return;
    finger = e.pointerId;
    zone.setPointerCapture(e.pointerId);
    const r = zone.getBoundingClientRect();
    home = { x: e.clientX, y: e.clientY };
    stick.style.left = e.clientX - r.left + "px";
    stick.style.top = e.clientY - r.top + "px";
    stick.classList.add("held");
    place(0, 0);
    e.preventDefault();
  });
  zone.addEventListener("pointermove", (e) => {
    if (e.pointerId !== finger) return;
    place(e.clientX - home.x, e.clientY - home.y);
    e.preventDefault();
  });
  for (const type of ["pointerup", "pointercancel", "lostpointercapture"]) {
    zone.addEventListener(type, (e) => {
      if (e.pointerId === finger) release();
    });
  }

  const held = new Map();
  function press(el, down) {
    const button = el.dataset.button;
    const w = wasm();
    if (!w || held.get(button) === down) return;
    held.set(button, down);
    el.classList.toggle("down", down);
    w.runity_pad_button(button, down);
    if (down && navigator.vibrate) navigator.vibrate(8);
  }
  for (const el of document.querySelectorAll("#pad [data-button]")) {
    el.addEventListener("pointerdown", (e) => {
      el.setPointerCapture(e.pointerId);
      press(el, true);
      e.preventDefault();
    });
    for (const type of ["pointerup", "pointercancel", "lostpointercapture"]) {
      el.addEventListener(type, () => press(el, false));
    }
    el.addEventListener("contextmenu", (e) => e.preventDefault());
  }

  window.runityPad = {
    // Nothing held over a change of screen: a stick let go of in the menu
    // does not keep the cook walking.
    reset() {
      if (finger !== null) release();
      for (const el of document.querySelectorAll("#pad [data-button]")) press(el, false);
    },
  };
})();
