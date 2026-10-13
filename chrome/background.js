"use strict";

// Shared by the Firefox (event page) and Chrome (service worker) builds;
// keep firefox/ and chrome/ copies identical
const api = globalThis.browser ?? globalThis.chrome;

const HOST = "qwen_translate";
const MENU_ID = "qt-translate-selection";
// The host keeps the model (1-4 GB RAM, by variant) loaded; release it after this long unused
const IDLE_MS = 5 * 60 * 1000;

let port = null;
let queue = [];
let busy = false;
let nextId = 1;
let idleTimer = null;

function sendToPanel(req, msg) {
  return api.tabs.sendMessage(req.tabId, msg, { frameId: req.frameId });
}

function notify(req, msg) {
  sendToPanel(req, { type: "qt-result", id: req.id, ...msg }).catch(() => {});
}

function ensurePort() {
  if (port) return true;
  try {
    const connection = api.runtime.connectNative(HOST);
    port = connection;
    connection.onMessage.addListener((msg) => {
      // A delayed event from a disconnected host must not consume a request
      // belonging to a newer connection.
      if (port !== connection) return;
      const req = queue.shift();
      busy = false;
      if (req) notify(req, { translation: msg.translation, error: msg.error });
      if (queue.length === 0) scheduleIdle();
      pump();
    });
    connection.onDisconnect.addListener((p) => {
      if (port !== connection) return;
      port = null;
      busy = false;
      const err = p.error || api.runtime.lastError;
      const message = err && err.message ? err.message : "翻译程序连接断开";
      const pending = queue;
      queue = [];
      for (const req of pending) notify(req, { error: message });
    });
  } catch (e) {
    port = null;
    return false;
  }
  return true;
}

function failPort(message) {
  const connection = port;
  port = null;
  busy = false;
  const pending = queue;
  queue = [];
  for (const req of pending) notify(req, { error: message });
  if (connection) {
    try { connection.disconnect(); } catch (e) { /* already disconnected */ }
  }
}

// Closing the port ends the host process (stdin EOF), freeing the model's memory;
// the next request starts it again
function scheduleIdle() {
  clearTimeout(idleTimer);
  idleTimer = setTimeout(() => {
    if (port && !busy && queue.length === 0) {
      port.disconnect();
      port = null;
    }
  }, IDLE_MS);
}

function pump() {
  if (busy || queue.length === 0) return;
  clearTimeout(idleTimer);
  if (!ensurePort()) {
    const pending = queue;
    queue = [];
    for (const req of pending) {
      notify(req, { error: "无法启动翻译程序（native host 未安装？）" });
    }
    return;
  }
  busy = true;
  try {
    port.postMessage({ id: queue[0].id, text: queue[0].text });
  } catch (e) {
    failPort(e && e.message ? e.message : "翻译程序连接断开");
  }
}

// Inject the panel script into the clicked frame only when needed (the menu click
// grants activeTab), instead of a content script on every page. Falls back to the
// top frame when the selection is in a frame we may not script.
async function injectPanel(tabId, frameId) {
  for (const id of frameId ? [frameId, 0] : [0]) {
    try {
      await api.scripting.executeScript({
        target: { tabId, frameIds: [id] },
        files: ["content.js"],
      });
      return id;
    } catch (e) {
      // cross-origin frame, or a restricted page (about:, addons.mozilla.org, …)
    }
  }
  return null;
}

// Menus persist across restarts; (re)create them on install/startup only
function createMenu() {
  Promise.resolve(api.contextMenus.removeAll()).then(() => {
    api.contextMenus.create({
      id: MENU_ID,
      title: "翻译选中内容",
      contexts: ["selection"],
    });
  });
}
api.runtime.onInstalled.addListener(createMenu);
api.runtime.onStartup.addListener(createMenu);

async function onMenuClicked(info, tab) {
  if (info.menuItemId !== MENU_ID || !tab || tab.id == null) return;
  const fallback = (info.selectionText || "").trim();
  if (!fallback) return;
  const selectionFrameId = info.frameId || 0;
  const frameId = await injectPanel(tab.id, selectionFrameId);
  if (frameId == null) return;
  const req = { id: nextId++, tabId: tab.id, frameId, text: fallback };
  // Only the original frame can recover line breaks from the DOM selection.
  // A fallback panel in the top frame may contain an unrelated old selection.
  const shown = await sendToPanel(req, {
    type: "qt-show", id: req.id, text: fallback,
    useSelection: frameId === selectionFrameId,
  }).catch(
    () => null,
  );
  // If the tab/frame disappeared after injection, there is no receiver for
  // the result. Do not spend a native-host request that can never be shown.
  if (shown == null) return;
  if (shown && shown.text) req.text = shown.text;
  // The new panel replaced the old one in this frame, so requests still waiting
  // for it are stale; drop them (the one in flight can't be cancelled)
  queue = queue.filter(
    (r, i) => (busy && i === 0) || r.tabId !== req.tabId || r.frameId !== req.frameId,
  );
  queue.push(req);
  pump();
}
api.contextMenus.onClicked.addListener(onMenuClicked);
