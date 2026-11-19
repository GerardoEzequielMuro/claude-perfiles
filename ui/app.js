"use strict";

const invoke = (cmd, args) => window.__TAURI__.core.invoke(cmd, args);
const $ = (id) => document.getElementById(id);

const COLOR_NAMES = { "#E07A55": "Terracota", "#E3A857": "Ámbar", "#9DBB7E": "Salvia", "#6FA8C9": "Cielo", "#D98AA0": "Rosa", "#A7A0C9": "Lavanda", "#7FB8B0": "Agua", "#B8A898": "Arena" };

let snap = null;          // último estado del backend
const accounts = {};      // nombre -> mail de Code (se actualiza en los scans profundos)
const used = {};          // nombre -> Desktop ya usado
let tick = 0;
let busy = false;
let claudeMissing = false; // solo se mide en los scans profundos

// ---------------------------------------------------------------- utilidades

function el(tag, props = {}, ...kids) {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (k === "class") e.className = v;
    else if (k === "style") Object.assign(e.style, v);
    else if (k.startsWith("on")) e.addEventListener(k.slice(2), v);
    else if (v === true) e.setAttribute(k, "");
    else if (v !== false && v != null) e.setAttribute(k, v);
  }
  for (const k of kids.flat()) if (k != null) e.append(k.nodeType ? k : document.createTextNode(k));
  return e;
}
function icon(name, cls = "") {
  const s = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  s.setAttribute("class", "ic " + cls);
  const u = document.createElementNS("http://www.w3.org/2000/svg", "use");
  u.setAttribute("href", "#i-" + name);
  s.append(u);
  return s;
}
function shade(hex, f) {
  const n = parseInt(hex.slice(1), 16);
  const r = Math.round(((n >> 16) & 255) * f), g = Math.round(((n >> 8) & 255) * f), b = Math.round((n & 255) * f);
  return `rgb(${r},${g},${b})`;
}

let toastTimer;
// long: avisos que hay que leer (terminal que falta, permisos) quedan más tiempo en pantalla
function toast(msg, err = false, long = false) {
  const t = $("toast");
  t.textContent = msg;
  t.classList.toggle("err", err);
  t.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (t.hidden = true), err || long ? 8000 : 3500);
}
async function run(cmd, args, okMsg) {
  try {
    const r = await invoke(cmd, args);
    if (okMsg) toast(okMsg);
    return r ?? true;
  } catch (e) {
    toast(String(e), true);
    return null;
  }
}

// ---------------------------------------------------------------- estado

async function refresh(deep = false) {
  if (busy) return;
  busy = true;
  try {
    snap = await invoke("snapshot", { deep });
    if (deep) {
      for (const p of snap.profiles) { accounts[p.name] = p.account; used[p.name] = p.desktop_used; }
      claudeMissing = !snap.claude_found;
    }
    render();
  } catch (e) {
    console.error(e);
  } finally {
    busy = false;
  }
}

function loop() {
  const deep = tick++ % 5 === 0;
  refresh(deep).finally(() => setTimeout(loop, document.hidden ? 6000 : 2200));
}

// ---------------------------------------------------------------- render

function render() {
  const s = snap;
  document.documentElement.dataset.os = s.os;

  // avisos persistentes: se ven hasta que se resuelve la causa
  const notes = [];
  if (s.terminal_missing) {
    const fallback = s.terminals.find((t) => t.id === s.terminal);
    notes.push(el("div", { class: "notice", role: "status" }, icon("alert"),
      el("div", { class: "notice-text" },
        el("strong", {}, `${s.terminal_missing} no está instalada en esta computadora.`),
        ` Mientras tanto Claude Code se abre en ${fallback ? fallback.label : "la terminal del sistema"}.`),
      el("button", { class: "btn soft", "data-key": "pick-terminal", onclick: (e) => terminalMenu(e.currentTarget) }, "Elegir terminal")));
  }
  if (!s.desktop_version) {
    notes.push(el("div", { class: "notice", role: "status" }, icon("alert"),
      el("div", { class: "notice-text" }, el("strong", {}, "No encuentro Claude Desktop."), " Los perfiles de Code funcionan igual."),
      el("button", { class: "btn soft", "data-key": "get-desktop", onclick: () => run("open_download", {}, "Abrí la descarga de Claude Desktop. Cuando lo instales, aparece acá solo.") }, "Descargar Claude Desktop")));
  }
  if (claudeMissing) {
    notes.push(el("div", { class: "notice", role: "status" }, icon("alert"),
      el("div", { class: "notice-text" }, el("strong", {}, "No encuentro Claude Code en esta computadora."), " Sin él, la terminal se abre pero no arranca Claude."),
      s.claude_install ? el("button", { class: "btn soft", "data-key": "install-claude", title: s.claude_install, onclick: () => installTool("claude", "Claude Code") }, "Instalar Claude Code") : null));
  }
  const box = $("notice");
  box.replaceChildren(...notes);
  box.hidden = notes.length === 0;

  const grid = $("grid");
  const focused = document.activeElement && document.activeElement.dataset.key;
  grid.replaceChildren(...s.profiles.map(card), addCard());
  if (focused) grid.querySelector(`[data-key="${CSS.escape(focused)}"]`)?.focus();

  const term = s.terminals.find((t) => t.id === s.terminal);
  const mode = term && term.tabs ? (s.new_window ? " · ventana nueva" : " · pestaña nueva") : "";
  $("termLabel").textContent = "Code se abre en " + (term ? term.label : "—") + mode;

  const open = s.profiles.filter((p) => p.desktop_running).length;
  $("footInfo").textContent =
    (s.desktop_version ? "Claude Desktop " + s.desktop_version : "Sin Claude Desktop") +
    "  ·  " + (open === 0 ? "ningún Desktop abierto" : open === 1 ? "1 Desktop abierto" : open + " Desktop abiertos") +
    "  ·  v" + s.version;

  $("autostart").checked = s.autostart;
  $("tray").checked = s.minimize_to_tray;
  $("trayLabel").textContent = s.os === "mac" ? "Cerrar a la barra de menú" : "Cerrar a la bandeja";
}

function card(p) {
  const color = p.color || "#E07A55";
  const acc = accounts[p.name];
  const wasUsed = used[p.name] ?? true;
  const desktopText = p.desktop_running ? "Abierto ahora" : wasUsed ? "Cerrado" : "Sin abrir todavía";
  const codeText = acc ? acc : accounts.hasOwnProperty(p.name) ? "Sin login todavía" : "…";

  return el("article", { class: "card", "aria-label": "Perfil " + p.name },
    el("div", { class: "card-head" },
      el("div", { class: "avatar", style: { background: `linear-gradient(135deg, ${color}, ${shade(color, 0.78)})` } }, p.name.slice(0, 1).toUpperCase()),
      el("div", { class: "card-title" },
        el("h3", { title: p.name }, p.name),
        el("p", {}, p.principal ? "Tu instalación de siempre" : "Perfil aislado")),
      el("button", { class: "icon-btn", title: "Más opciones", "aria-haspopup": "menu", "data-key": "menu:" + p.name, onclick: (e) => profileMenu(e.currentTarget, p) }, icon("more"))),
    el("div", { class: "rows" },
      el("div", { class: "row" },
        icon("monitor", "lead"),
        el("div", { class: "row-text" },
          el("div", { class: "k" }, "Claude Desktop"),
          el("div", { class: "v status" }, el("span", { class: "dot" + (p.desktop_running ? " on" : "") }), desktopText)),
        el("button", { class: "btn " + (p.desktop_running ? "ghost" : "soft"), "data-key": "d:" + p.name, disabled: !snap.desktop_version, onclick: () => openDesktop(p) },
          p.desktop_running ? "Mostrar" : "Abrir")),
      el("div", { class: "row" },
        icon("terminal", "lead"),
        el("div", { class: "row-text" },
          el("div", { class: "k" }, "Claude Code"),
          el("div", { class: "v", title: codeText }, codeText),
          el("button", { class: "folder", title: "Carpeta donde arranca Code: " + (p.folder || "tu carpeta de usuario") + ". Clic para cambiarla.", onclick: () => pickFolder(p) },
            icon("folder", "sm"), el("span", { class: p.folder ? "path" : "" }, p.folder ? p.folder_short : "Arranca en tu carpeta de usuario"))),
        el("button", { class: "btn soft", "data-key": "c:" + p.name, onclick: () => openCode(p) }, "Abrir"))));
}

function addCard() {
  return el("button", { class: "add-card", "data-key": "add", onclick: openNew },
    el("div", { class: "plus" }, icon("plus")),
    el("strong", {}, "Agregar perfil"),
    el("small", {}, "Otra cuenta de Claude, separada de las demás"));
}

// ---------------------------------------------------------------- acciones

async function openDesktop(p) {
  const first = !(used[p.name] ?? true);
  const ok = await run("open_desktop", { name: p.name });
  if (ok && !p.desktop_running) toast(first ? `Abriendo Desktop · ${p.name}. Iniciá sesión con la cuenta de este perfil.` : `Abriendo Desktop · ${p.name}`);
  setTimeout(() => refresh(true), 1500);
}
async function openCode(p) {
  try {
    const note = await invoke("open_code", { name: p.name });
    if (note) toast(note, false, true);
    else toast(accounts[p.name] ? `Abriendo Code · ${p.name}` : `Abriendo Code · ${p.name}. La primera vez escribí /login.`);
  } catch (e) {
    toast(String(e), true);
  }
  setTimeout(() => refresh(true), 4000);
}
// Instala en una ventana visible (o abre la descarga) y después vigila hasta que aparezca,
// para avisar sin que haga falta reiniciar la app.
const watching = new Set();
async function installTool(id, label) {
  let msg;
  try { msg = await invoke("install_tool", { id }); } catch (e) { toast(String(e), true); return; }
  toast(msg, false, true);
  if (watching.has(id)) return;
  watching.add(id);
  const isReady = () => id === "claude" ? !claudeMissing : !!snap.terminals.find((t) => t.id === id && t.available);
  const started = Date.now();
  const timer = setInterval(async () => {
    await invoke("rescan_terminals").catch(() => {});
    await refresh(true);
    if (isReady()) {
      clearInterval(timer); watching.delete(id);
      const usable = id !== "claude" && snap.terminal !== id;
      toast(usable ? `${label} quedó instalada. Elegila en el menú de terminales, abajo a la izquierda.` : `${label} quedó instalado.`, false, true);
    } else if (Date.now() - started > 10 * 60 * 1000) {
      clearInterval(timer); watching.delete(id);
    }
  }, 5000);
}

async function pickFolder(p) {
  const changed = await run("pick_folder", { name: p.name });
  if (changed) refresh();
}

// ---------------------------------------------------------------- menú flotante

const menu = $("menu");
let menuAnchor = null;

function showMenu(anchor, items, placement = "below") {
  menu.replaceChildren(...items);
  menu.hidden = false;
  menuAnchor = anchor;
  const r = anchor.getBoundingClientRect();
  const m = menu.getBoundingClientRect();
  let left = placement === "above" ? r.left : r.right - m.width;
  let top = placement === "above" ? r.top - m.height - 8 : r.bottom + 6;
  left = Math.max(10, Math.min(left, innerWidth - m.width - 10));
  if (top + m.height > innerHeight - 10) top = r.top - m.height - 6;
  menu.style.left = left + "px";
  menu.style.top = Math.max(10, top) + "px";
  menu.querySelector("button:not([disabled])")?.focus();
}
function hideMenu() {
  if (menu.hidden) return;
  menu.hidden = true;
  menuAnchor?.focus();
  menuAnchor = null;
}
function mItem(label, ico, act, opts = {}) {
  return el("button", { role: "menuitem", class: opts.danger ? "danger" : "", disabled: opts.disabled, onclick: () => { hideMenu(); act(); } },
    opts.tick !== undefined ? el("span", { class: "tick" }, opts.tick ? icon("check", "sm") : "") : icon(ico),
    el("span", {}, label),
    opts.hint ? el("span", { class: "hint" }, opts.hint) : null);
}
const mSep = () => el("div", { class: "sep" });
const mLabel = (t) => el("div", { class: "label" }, t);

function profileMenu(anchor, p) {
  const items = [
    mItem("Cambiar carpeta de Code…", "folder", () => pickFolder(p)),
    p.folder ? mItem("Volver a la carpeta de usuario", "folder", async () => { await run("reset_folder", { name: p.name }); refresh(); }) : null,
    mItem("Cambiar color", "palette", () => openRecolor(p)),
    snap.os === "windows" ? mItem("Atajos en el Escritorio", "link", () => run("shortcuts", { name: p.name }, "Atajos creados en el Escritorio")) : null,
    mItem(snap.os === "mac" ? "Mostrar datos en Finder" : "Abrir carpeta de datos", "box", () => run("open_data", { name: p.name })),
    mSep(),
    mItem("Cerrar Desktop", "power", async () => { await run("stop_desktop", { name: p.name }, "Cerrando Desktop · " + p.name); setTimeout(() => refresh(true), 1200); }, { disabled: !p.desktop_running }),
    p.principal ? null : mItem("Borrar perfil…", "trash", () => askDelete(p), { danger: true }),
  ].filter(Boolean);
  showMenu(anchor, items);
}

async function terminalMenu(anchor) {
  // volver a detectar: puede haber instalado o desinstalado una terminal con la app abierta
  await invoke("rescan_terminals").catch(() => {});
  await refresh();
  const s = snap;
  const cur = s.terminals.find((t) => t.id === s.terminal);
  const items = [mLabel("Claude Code se abre en")];
  for (const t of s.terminals.filter((t) => t.available)) {
    const inUse = t.id === s.terminal;
    const hint = inUse && s.terminal_missing ? "se usa mientras tanto" : t.hint;
    items.push(mItem(t.label, null, async () => { await run("set_terminal", { id: t.id }, `Listo: Code se abre en ${t.label}.`); refresh(); },
      { tick: inUse && !s.terminal_missing, hint }));
  }
  const missing = s.terminals.filter((t) => !t.available);
  if (missing.length) {
    items.push(mSep(), mLabel("No instaladas"));
    for (const t of missing) {
      const saved = s.terminal_missing === t.label ? "la elegida · " : "";
      items.push(t.install
        ? mItem(`Instalar ${t.label}`, "download", () => installTool(t.id, t.label), { hint: saved + t.install })
        : mItem(t.label, "download", () => {}, { disabled: true, hint: "no instalada" }));
    }
  }
  if (cur && cur.tabs) {
    items.push(mSep());
    items.push(mItem("Pestaña nueva en la ventana abierta", null, async () => { await run("set_new_window", { on: false }); refresh(); }, { tick: !s.new_window }));
    items.push(mItem("Ventana nueva cada vez", null, async () => { await run("set_new_window", { on: true }); refresh(); }, { tick: s.new_window }));
  }
  showMenu(anchor, items, "above");
}

document.addEventListener("mousedown", (e) => { if (!menu.hidden && !menu.contains(e.target) && e.target.closest("button") !== menuAnchor) hideMenu(); });
window.addEventListener("blur", hideMenu);
window.addEventListener("resize", hideMenu);
menu.addEventListener("keydown", (e) => {
  const btns = [...menu.querySelectorAll("button:not([disabled])")];
  const i = btns.indexOf(document.activeElement);
  if (e.key === "ArrowDown") { e.preventDefault(); btns[(i + 1) % btns.length]?.focus(); }
  if (e.key === "ArrowUp") { e.preventDefault(); btns[(i - 1 + btns.length) % btns.length]?.focus(); }
});

// ---------------------------------------------------------------- diálogo nuevo / color

let dlgMode = null; // {kind:"new"} | {kind:"color", p}
let dlgColor = null;

function renderSwatches() {
  const box = $("swatches");
  box.replaceChildren(...snap.palette.map((hex) =>
    el("button", { type: "button", class: "swatch", role: "radio", "aria-checked": String(hex === dlgColor), "aria-label": COLOR_NAMES[hex] || hex, title: COLOR_NAMES[hex] || hex,
      style: { background: hex }, onclick: () => { dlgColor = hex; renderSwatches(); box.querySelector('[aria-checked="true"]')?.focus(); } }, icon("check"))));
}
function openDialog() {
  $("dlgError").textContent = "";
  $("dlgScrim").hidden = false;
  renderSwatches();
}
function closeDialog() { $("dlgScrim").hidden = true; dlgMode = null; }

function openNew() {
  if (!snap) return;
  dlgMode = { kind: "new" };
  const usedColors = new Set(snap.profiles.map((p) => (p.color || "").toUpperCase()));
  dlgColor = snap.palette.slice(1).find((c) => !usedColors.has(c)) || snap.palette[1];
  $("dlgTitle").textContent = "Nuevo perfil";
  $("dlgHint").textContent = "Otra cuenta de Claude, con su propio login en Desktop y en Code.";
  $("dlgOk").textContent = "Crear perfil";
  $("nameRow").hidden = false;
  $("seedRow").hidden = false;
  $("nameInput").value = "";
  $("seedInput").checked = false;
  openDialog();
  setTimeout(() => $("nameInput").focus(), 0);
}
function openRecolor(p) {
  dlgMode = { kind: "color", p };
  dlgColor = (p.color || "#E07A55").toUpperCase();
  $("dlgTitle").textContent = "Color de «" + p.name + "»";
  $("dlgHint").textContent = "Para reconocer cada cuenta de un vistazo. También tiñe la pestaña de la terminal.";
  $("dlgOk").textContent = "Guardar";
  $("nameRow").hidden = true;
  $("seedRow").hidden = true;
  openDialog();
  setTimeout(() => $("swatches").querySelector('[aria-checked="true"]')?.focus(), 0);
}

$("dlg").addEventListener("submit", async (e) => {
  e.preventDefault();
  if (!dlgMode) return;
  if (dlgMode.kind === "color") {
    await run("set_color", { name: dlgMode.p.name, color: dlgColor });
    closeDialog();
    refresh();
    return;
  }
  const name = $("nameInput").value.trim();
  try {
    $("dlgOk").disabled = true;
    await invoke("create_profile", { name, color: dlgColor, seed: $("seedInput").checked });
    closeDialog();
    toast(`Listo: «${name}». Abrí Desktop o Code y entrá con esa cuenta.`);
    tick = 0;
    refresh(true);
  } catch (err) {
    $("dlgError").textContent = String(err);
  } finally {
    $("dlgOk").disabled = false;
  }
});
$("nameInput").addEventListener("input", () => ($("dlgError").textContent = ""));
$("dlgCancel").addEventListener("click", closeDialog);
$("dlgScrim").addEventListener("mousedown", (e) => { if (e.target === e.currentTarget) closeDialog(); });

// ---------------------------------------------------------------- borrar

let pendingDelete = null;
function askDelete(p) {
  pendingDelete = p;
  $("delTitle").textContent = "¿Borrar «" + p.name + "»?";
  $("delScrim").hidden = false;
  setTimeout(() => $("delCancel").focus(), 0);
}
function closeDelete() { $("delScrim").hidden = true; pendingDelete = null; }
$("delCancel").addEventListener("click", closeDelete);
$("delScrim").addEventListener("mousedown", (e) => { if (e.target === e.currentTarget) closeDelete(); });
$("delOk").addEventListener("click", async () => {
  const p = pendingDelete;
  closeDelete();
  if (!p) return;
  toast("Borrando «" + p.name + "»…");
  const ok = await run("delete_profile", { name: p.name });
  if (ok) toast("«" + p.name + "» borrado.");
  delete accounts[p.name];
  refresh(true);
});

// ---------------------------------------------------------------- footer + teclado

$("newBtn").addEventListener("click", openNew);
$("termBtn").addEventListener("click", (e) => terminalMenu(e.currentTarget));
$("autostart").addEventListener("change", async (e) => {
  const on = e.target.checked;
  const ok = await run("set_autostart", { on }, on ? "Se va a abrir sola al iniciar sesión, en segundo plano." : "Ya no se abre sola.");
  if (!ok) e.target.checked = !on;
});
$("tray").addEventListener("change", (e) => run("set_tray", { on: e.target.checked }));

document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") {
    if (!menu.hidden) hideMenu();
    else if (!$("delScrim").hidden) closeDelete();
    else if (!$("dlgScrim").hidden) closeDialog();
  }
  const mod = e.ctrlKey || e.metaKey;
  if (mod && e.key.toLowerCase() === "n") { e.preventDefault(); openNew(); }
  if (mod && e.key.toLowerCase() === "r") e.preventDefault(); // sin recargar la webview
});
document.addEventListener("contextmenu", (e) => { if (!e.target.closest("input")) e.preventDefault(); });
document.addEventListener("visibilitychange", async () => {
  if (document.hidden) return;
  await invoke("rescan_terminals").catch(() => {});
  if (snap && !snap.desktop_version) await invoke("refresh_desktop").catch(() => {}); // por si lo instaló recién
  refresh(true);
});
// avisos de acciones que no salieron de la ventana (bandeja, atajos del Escritorio)
window.__TAURI__.event.listen("notice", (e) => {
  toast(e.payload.message, e.payload.error, true);
  refresh(true);
});

loop();
