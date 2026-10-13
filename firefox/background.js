"use strict";

const HOST = "qwen_translate";
let port = null;
let queue = [];
let busy = false;
let nextId = 1;

function notify(req, msg) {
  browser.tabs
    .sendMessage(req.tabId, { type: "qt-result", id: req.id, ...msg })
    .catch(() => {});
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

browser.contextMenus.create({
  id: "qt-translate-selection",
  title: "翻译选中内容",
  contexts: ["selection"],
});

browser.contextMenus.onClicked.addListener((info, tab) => {
  if (info.menuItemId !== "qt-translate-selection" || !tab || tab.id == null) return;
  const text = (info.selectionText || "").trim();
  if (!text) return;
  const req = { id: nextId++, tabId: tab.id, text };
  browser.tabs.sendMessage(tab.id, { type: "qt-show", id: req.id, text }).catch(() => {});
  queue.push(req);
  pump();
});
