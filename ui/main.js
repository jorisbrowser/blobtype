"use strict";

const blob = document.getElementById("blob");
const msg = document.getElementById("msg");
const hints = document.getElementById("hints");
const canvas = document.getElementById("wave");
const ctx = canvas.getContext("2d");

// Size the canvas buffer to its CSS box so the bars stay crisp on scaled displays.
const WAVE_W = 304;
const WAVE_H = 44;
canvas.width = Math.round(WAVE_W * devicePixelRatio);
canvas.height = Math.round(WAVE_H * devicePixelRatio);

const BARS = 40;
const levels = new Array(BARS).fill(0);
let target = 0;
let smooth = 0;
let raf = 0;
let phase = 0;

function draw() {
  smooth += (target - smooth) * 0.45;
  phase += 0.35;
  // Scroll the history left and append a new value with a little life in it.
  levels.shift();
  levels.push(Math.min(1, smooth * (0.85 + 0.15 * Math.sin(phase))));
  const w = canvas.width;
  const h = canvas.height;
  ctx.clearRect(0, 0, w, h);
  ctx.fillStyle = "#f4f4f5";
  const step = w / BARS;
  const barW = step * 0.6;
  for (let i = 0; i < BARS; i++) {
    const v = levels[i];
    const bh = Math.max(barW, v * h);
    const x = i * step + (step - barW) / 2;
    const y = (h - bh) / 2;
    ctx.beginPath();
    ctx.roundRect(x, y, barW, bh, barW / 2);
    ctx.fill();
  }
  raf = requestAnimationFrame(draw);
}

function startWave() {
  if (!raf) raf = requestAnimationFrame(draw);
}
function stopWave() {
  cancelAnimationFrame(raf);
  raf = 0;
  levels.fill(0);
  target = smooth = 0;
}

let audio = null;
function beep(freq, ms) {
  try {
    audio = audio || new AudioContext();
    const o = audio.createOscillator();
    const g = audio.createGain();
    o.frequency.value = freq;
    g.gain.setValueAtTime(0.0001, audio.currentTime);
    g.gain.exponentialRampToValueAtTime(0.08, audio.currentTime + 0.01);
    g.gain.exponentialRampToValueAtTime(0.0001, audio.currentTime + ms / 1000);
    o.connect(g).connect(audio.destination);
    o.start();
    o.stop(audio.currentTime + ms / 1000 + 0.02);
  } catch (_) {
    /* sounds are optional */
  }
}

const KEY_NAMES = { Escape: "Esc", Super: "Win", Control: "Ctrl" };

function renderHints(list) {
  hints.replaceChildren(
    ...list.map(({ keys, label }) => {
      const row = document.createElement("div");
      row.className = "hint";
      for (const k of keys.split("+")) {
        const kbd = document.createElement("kbd");
        kbd.textContent = KEY_NAMES[k] || k;
        row.append(kbd);
      }
      const span = document.createElement("span");
      span.textContent = label;
      row.append(span);
      return row;
    }),
  );
}

function setState({ state, message, sound, hints: list }) {
  if (state === "hidden") {
    blob.classList.add("hidden");
    hints.classList.add("hidden");
    setTimeout(stopWave, 450);
    return;
  }
  msg.textContent = message || "";
  blob.dataset.state = state;
  renderHints(list || []);
  // Un-hide on the next frame so the slide-in transition always runs.
  requestAnimationFrame(() => {
    blob.classList.remove("hidden");
    hints.classList.toggle("hidden", !hints.childElementCount);
  });
  if (state === "recording") {
    startWave();
    if (sound) beep(880, 90);
  } else {
    stopWave();
    if (sound && state === "transcribing") beep(520, 90);
    if (sound && state === "error") beep(220, 160);
  }
}

const { listen } = window.__TAURI__.event;
listen("blob-state", (e) => setState(e.payload));
listen("blob-level", (e) => {
  target = Math.max(0, Math.min(1, e.payload));
});
