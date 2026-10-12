"use strict";

const HOST = "qwen_translate";
const MENU_ID = "qt-translate-selection";

let port = null;
let queue = [];
let busy = false;
let nextId = 1;

function sendToPanel(req, msg) {
  return browser.tabs.sendMessage(req.tabId, msg, { frameId: req.frameId });
}

function notify(req, msg) {
  sendToPanel(req, { type: "qt-result", id: req.id, ...msg }).catch(() => {});
}

function ensurePort() {
  if (port) return true;
  try {
    port = browser.runtime.connectNative(HOST);
  } catch (e) {
    return false;
  }
  port.onMessage.addListener((msg) => {
    const req = queue.shift();
    busy = false;
    if (req) notify(req, { translation: msg.translation, error: msg.error });
    pump();
  });
  port.onDisconnect.addListener((p) => {
    // Firefox reports why the host went away on the port itself
    const err = p.error || browser.runtime.lastError;
    port = null;
    busy = false;
    const message = err && err.message ? err.message : "翻译程序连接断开";
    const pending = queue;
    queue = [];
    for (const req of pending) notify(req, { error: message });
  });
  return true;
}

function pump() {
  if (busy || queue.length === 0) return;
  if (!ensurePort()) {
    const pending = queue;
    queue = [];
    for (const req of pending) {
      notify(req, { error: "无法启动翻译程序（native host 未安装？）" });
    }
    return;
  }
  busy = true;
  port.postMessage({ id: queue[0].id, text: queue[0].text });
}

// Inject the panel script into the clicked frame only when needed (the menu click
// grants activeTab), instead of a content script on every page. Falls back to the
// top frame when the selection is in a frame we may not script.
async function injectPanel(tabId, frameId) {
  for (const id of frameId ? [frameId, 0] : [0]) {
    try {
      await browser.scripting.executeScript({
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
  browser.contextMenus.removeAll().then(() => {
    browser.contextMenus.create({
      id: MENU_ID,
      title: "翻译选中内容",
      contexts: ["selection"],
    });
  });
}
browser.runtime.onInstalled.addListener(createMenu);
browser.runtime.onStartup.addListener(createMenu);

browser.contextMenus.onClicked.addListener(async (info, tab) => {
  if (info.menuItemId !== MENU_ID || !tab || tab.id == null) return;
  const fallback = (info.selectionText || "").trim();
  if (!fallback) return;
  const frameId = await injectPanel(tab.id, info.frameId || 0);
  if (frameId == null) return;
  const req = { id: nextId++, tabId: tab.id, frameId, text: fallback };
  // The panel replies with the page's own selection, which keeps line breaks
  // (selectionText collapses them and may be truncated)
  const shown = await sendToPanel(req, { type: "qt-show", id: req.id, text: fallback }).catch(
    () => null,
  );
  if (shown && shown.text) req.text = shown.text;
  queue.push(req);
  pump();
});
