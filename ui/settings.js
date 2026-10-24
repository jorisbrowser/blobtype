"use strict";

const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);

let state = null; // from get_settings
let saved = ""; // snapshot of the last saved form, to detect changes
const byId = (id) => state.providers.find((p) => p.id === id);

function say(el, text, kind) {
  el.textContent = text || "";
  el.className = `status ${kind || ""}`;
}

// ---------------------------------------------------------------- shortcuts
// Tauri accepts "Ctrl+Alt+D", "Shift+F9", "Super+Space", ...
function comboFromEvent(e) {
  const mods = [];
  if (e.ctrlKey) mods.push("Ctrl");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  if (e.metaKey) mods.push("Super");
  let key = null;
  const c = e.code;
  if (/^Key[A-Z]$/.test(c)) key = c.slice(3);
  else if (/^Digit\d$/.test(c)) key = c.slice(5);
  else if (/^F([1-9]|1[0-9]|2[0-4])$/.test(c)) key = c;
  else if (c === "Space") key = "Space";
  return { mods, key };
}

function hotkeyHtml(combo) {
  if (!combo) return "Off";
  return combo
    .split("+")
    .map((k) => `<kbd>${k.replace(/[<>&]/g, "")}</kbd>`)
    .join(' <span class="sep">+</span> ');
}

function showHotkey() {
  document.querySelectorAll("[data-hotkey]").forEach((el) => (el.innerHTML = hotkeyHtml(state.settings.hotkey)));
  $("hotkey").innerHTML = hotkeyHtml(form.hotkey);
  $("last-hotkey").innerHTML = hotkeyHtml(form.lastHotkey);
}

const form = {}; // current values of the settings page

// `field` is the form property the recorded shortcut goes to. Only optional
// shortcuts can be turned off with Backspace.
function startCapture(id, field, optional) {
  const btn = $(id);
  btn.classList.add("listening");
  btn.textContent = "Press the new shortcut…";
  invoke("suspend_hotkey", { suspended: true });
  const onKey = (e) => {
    e.preventDefault();
    if (e.key === "Escape") return stop();
    if (optional && e.key === "Backspace") {
      form[field] = "";
      stop();
      return changed();
    }
    const { mods, key } = comboFromEvent(e);
    if (!key) {
      btn.textContent = mods.length ? `${mods.join(" + ")} + …` : "Press the new shortcut…";
      return;
    }
    if (!mods.length && !/^F\d+$/.test(key)) {
      btn.textContent = "Add Ctrl, Alt, Shift or Win (or use an F-key)";
      return;
    }
    form[field] = [...mods, key].join("+");
    stop();
    changed();
  };
  const stop = () => {
    window.removeEventListener("keydown", onKey, true);
    btn.removeEventListener("blur", stop);
    btn.classList.remove("listening");
    invoke("suspend_hotkey", { suspended: false });
    showHotkey();
  };
  window.addEventListener("keydown", onKey, true);
  btn.addEventListener("blur", stop);
}

// ---------------------------------------------------------------- settings page
function fillSettings() {
  const s = state.settings;
  Object.assign(form, s);
  const provider = $("provider");
  provider.innerHTML = "";
  for (const p of state.providers) provider.add(new Option(p.label, p.id));
  provider.value = s.provider;

  const mic = $("microphone");
  mic.innerHTML = "";
  mic.add(new Option("System default", ""));
  for (const name of state.microphones) mic.add(new Option(name, name));
  if (s.microphone && !state.microphones.includes(s.microphone)) {
    mic.add(new Option(`${s.microphone} (not connected)`, s.microphone));
  }
  mic.value = s.microphone;

  const lang = $("language");
  if (![...lang.options].some((o) => o.value === s.language)) lang.add(new Option(s.language, s.language));
  lang.value = s.language;

  const max = $("max");
  if (![...max.options].some((o) => +o.value === s.maxRecordingSecs)) {
    max.add(new Option(`${s.maxRecordingSecs} seconds`, String(s.maxRecordingSecs)));
  }
  max.value = String(s.maxRecordingSecs);

  $("paste").value = String(s.paste);
  $("live").checked = s.live;
  $("sounds").checked = s.sounds;
  $("discord").checked = s.discordMute;
  const dkey = $("discord-key");
  if (!dkey.options.length) for (let i = 13; i <= 24; i++) dkey.add(new Option(`F${i}`, `F${i}`));
  if (![...dkey.options].some((o) => o.value === s.discordKey)) dkey.add(new Option(s.discordKey, s.discordKey));
  dkey.value = s.discordKey;
  $("discord-setup").hidden = !s.discordMute;
  $("autostart").checked = state.autostart;
  $("model").value = s.model;
  $("base-url").value = s.baseUrl;
  $("key").value = "";
  showHotkey();
  providerChanged();
  saved = snapshot();
  changed();
}

function readForm() {
  form.provider = $("provider").value;
  form.model = $("model").value.trim();
  form.baseUrl = $("base-url").value.trim();
  form.language = $("language").value;
  form.live = $("live").checked;
  form.paste = $("paste").value === "true";
  form.microphone = $("microphone").value;
  form.sounds = $("sounds").checked;
  form.discordMute = $("discord").checked;
  form.discordKey = $("discord-key").value;
  form.maxRecordingSecs = +$("max").value;
}

const snapshot = () => {
  readForm();
  return JSON.stringify({ ...form, autostart: $("autostart").checked, key: $("key").value.trim() });
};

function changed() {
  const dirty = snapshot() !== saved;
  $("save").disabled = !dirty;
  if (dirty) say($("status"), "Unsaved changes");
}

async function providerChanged() {
  const p = byId($("provider").value);
  $("provider-note").textContent = p.note;
  $("url-field").hidden = !p.selfHosted;
  $("get-key").parentElement.hidden = !p.keyUrl;
  $("get-key").textContent = p.keyUrl ? `Get a key at ${new URL(p.keyUrl).hostname}` : "";
  const live = $("live");
  live.disabled = !p.live;
  if (!p.live) live.checked = false;
  $("live-hint").textContent = p.live
    ? "Words appear while you speak."
    : "Words appear while you speak. Available with OpenAI and Deepgram.";
  const hint = await invoke("key_hint", { provider: p.id });
  $("key").placeholder = hint
    ? `Saved (${hint}). Paste a new key to replace it.`
    : p.selfHosted
      ? "Optional"
      : "Paste your API key";
  if (p.id === state.settings.provider) updateReadyLine(hint);
}

function updateReadyLine(hint) {
  const p = byId(state.settings.provider);
  const ok = hint || p.selfHosted;
  $("ready-line").innerHTML = ok
    ? `Ready. Press ${hotkeyHtml(state.settings.hotkey)} anywhere to dictate.`
    : "Add an API key below to start dictating.";
}

async function save() {
  readForm();
  $("save").disabled = true;
  say($("status"), "Saving…");
  try {
    await invoke("save_settings", {
      settings: { ...form },
      apiKey: $("key").value,
      autostart: $("autostart").checked,
    });
    state = await invoke("get_settings");
    fillSettings();
    say($("status"), "Saved", "ok");
  } catch (e) {
    say($("status"), String(e), "err");
    $("save").disabled = false;
  }
}

async function sendDiscordKey() {
  const btn = $("discord-send");
  btn.disabled = true;
  say($("status"), "Switch to Discord, the key is pressed in 3 seconds…");
  try {
    await invoke("send_discord_key", { key: $("discord-key").value });
    say($("status"), `Sent ${$("discord-key").value}.`, "ok");
  } catch (e) {
    say($("status"), String(e), "err");
  } finally {
    btn.disabled = false;
  }
}

async function testKey() {
  readForm();
  $("test").disabled = true;
  say($("status"), "Testing…");
  try {
    say($("status"), await invoke("test_key", { provider: form.provider, apiKey: $("key").value, model: form.model, baseUrl: form.baseUrl }), "ok");
  } catch (e) {
    say($("status"), String(e), "err");
  } finally {
    $("test").disabled = false;
  }
}

// ---------------------------------------------------------------- first-run guide
const guide = { step: 0, provider: "groq" };

function showStep(n) {
  guide.step = n;
  document.querySelectorAll(".step").forEach((s) => s.classList.toggle("active", +s.dataset.step === n));
  document.querySelectorAll(".progress i").forEach((b, i) => b.classList.toggle("on", i <= n));
  if (n === 2) prepareConnect();
  if (n === 3) {
    showHotkey();
    $("g-try").focus();
  }
}

function renderCards() {
  const box = $("provider-cards");
  box.innerHTML = "";
  for (const p of state.providers) {
    const b = document.createElement("button");
    b.type = "button";
    b.className = "card";
    b.setAttribute("role", "radio");
    b.setAttribute("aria-checked", String(p.id === guide.provider));
    b.innerHTML = `<span><b></b><small></small></span>${p.id === "groq" ? '<span class="badge">Recommended</span>' : ""}`;
    b.querySelector("b").textContent = p.label;
    b.querySelector("small").textContent = p.note;
    b.addEventListener("click", () => {
      guide.provider = p.id;
      renderCards();
    });
    b.addEventListener("dblclick", () => showStep(2));
    box.appendChild(b);
  }
}

function prepareConnect() {
  const p = byId(guide.provider);
  $("connect-title").textContent = `Connect ${p.label}`;
  $("howto-key").hidden = p.selfHosted;
  $("g-hosted").hidden = !p.selfHosted;
  $("g-open-key").textContent = p.keyUrl ? `Open ${new URL(p.keyUrl).hostname}` : "";
  $("g-skip").hidden = true;
  say($("g-status"), "");
  setTimeout(() => (p.selfHosted ? $("g-url") : $("g-key")).focus(), 50);
}

function guideSettings() {
  const p = byId(guide.provider);
  return {
    ...state.settings,
    provider: p.id,
    baseUrl: p.selfHosted ? $("g-url").value.trim() : state.settings.baseUrl,
    live: state.settings.live && p.live,
  };
}

const guideKey = () => (byId(guide.provider).selfHosted ? $("g-hosted-key").value : $("g-key").value).trim();

async function guideSave() {
  await invoke("save_settings", { settings: guideSettings(), apiKey: guideKey(), autostart: true });
  state = await invoke("get_settings");
  showStep(3);
}

async function connect() {
  const p = byId(guide.provider);
  const s = guideSettings();
  if (!p.selfHosted && !guideKey()) return say($("g-status"), "Paste your key first.", "err");
  if (p.selfHosted && !s.baseUrl) return say($("g-status"), "Enter your server address first.", "err");
  $("g-connect").disabled = true;
  say($("g-status"), "Checking your key…");
  try {
    await invoke("test_key", { provider: p.id, apiKey: guideKey(), model: s.model, baseUrl: s.baseUrl });
    await guideSave();
  } catch (e) {
    say($("g-status"), String(e), "err");
    $("g-skip").hidden = false;
  } finally {
    $("g-connect").disabled = false;
  }
}

// ---------------------------------------------------------------- wiring
function wire() {
  document.querySelectorAll("[data-next]").forEach((b) => b.addEventListener("click", () => showStep(guide.step + 1)));
  document.querySelectorAll("[data-back]").forEach((b) => b.addEventListener("click", () => showStep(guide.step - 1)));
  $("g-open-key").addEventListener("click", () => invoke("open_key_page", { provider: guide.provider }));
  $("g-connect").addEventListener("click", connect);
  $("g-key").addEventListener("keydown", (e) => e.key === "Enter" && connect());
  $("g-skip").addEventListener("click", () => guideSave().catch((e) => say($("g-status"), String(e), "err")));
  $("g-finish").addEventListener("click", () => invoke("close_settings"));

  for (const id of ["model", "base-url", "key"]) $(id).addEventListener("input", changed);
  for (const id of ["language", "paste", "live", "microphone", "max", "sounds", "discord", "discord-key", "autostart"]) $(id).addEventListener("change", changed);
  $("discord").addEventListener("change", () => ($("discord-setup").hidden = !$("discord").checked));
  $("discord-send").addEventListener("click", sendDiscordKey);
  $("provider").addEventListener("change", async () => {
    await providerChanged();
    changed();
  });
  $("hotkey").addEventListener("click", () => startCapture("hotkey", "hotkey", false));
  $("last-hotkey").addEventListener("click", () => startCapture("last-hotkey", "lastHotkey", true));
  $("test").addEventListener("click", testKey);
  $("save").addEventListener("click", save);
  $("get-key").addEventListener("click", (e) => {
    e.preventDefault();
    invoke("open_key_page", { provider: $("provider").value });
  });
  $("config").addEventListener("click", (e) => {
    e.preventDefault();
    invoke("open_config_file");
  });
  $("rerun").addEventListener("click", (e) => {
    e.preventDefault();
    openGuide();
  });
  window.addEventListener("keydown", (e) => {
    if ((e.ctrlKey || e.metaKey) && e.key === "s" && !$("settings").hidden) {
      e.preventDefault();
      if (!$("save").disabled) save();
    }
  });
}

function openGuide() {
  guide.provider = state.settings.provider;
  renderCards();
  $("settings").hidden = true;
  $("guide").hidden = false;
  showStep(0);
}

async function init() {
  state = await invoke("get_settings");
  wire();
  fillSettings();
  if (state.firstRun) openGuide();
  else $("settings").hidden = false;
}

init().catch((e) => {
  document.body.textContent = `Could not load settings: ${e}`;
});
