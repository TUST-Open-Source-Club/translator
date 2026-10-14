"use strict";

(() => {
  if (window.__qwenTranslateInjected) return;
  window.__qwenTranslateInjected = true;

  // Shared by the Firefox and Chrome builds; keep firefox/ and chrome/ copies identical
  const api = globalThis.browser ?? globalThis.chrome;

  // Panel styles live in a shadow root, so page CSS can't restyle the panel and
  // the panel's CSS can't leak into the page. `all: initial` on the panel also cuts
  // off inherited page styles; !important on :host beats page !important rules.
  const STYLE = `
    :host {
      all: initial !important;
      display: block !important;
      position: absolute !important;
      z-index: 2147483647 !important;
      left: var(--qt-left) !important;
      top: var(--qt-top) !important;
    }
    .panel {
      all: initial;
      display: block;
      box-sizing: border-box;
      width: 400px;
      max-width: 90vw;
      background: #23232a;
      color: #e8e8ec;
      border: 1px solid #3a3a44;
      border-radius: 10px;
      box-shadow: 0 8px 30px rgba(0, 0, 0, 0.45);
      font: 14px/1.55 system-ui, "PingFang SC", "Microsoft YaHei", "Noto Sans CJK SC", sans-serif;
      text-align: left;
    }
    .header {
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding: 8px 12px;
      font-weight: 600;
      border-bottom: 1px solid #3a3a44;
    }
    button {
      font: inherit;
      cursor: pointer;
      border: none;
    }
    .close {
      background: none;
      color: #9a9aa5;
      font-size: 18px;
      line-height: 1;
      padding: 0 4px;
    }
    .close:hover { color: #fff; }
    .body { padding: 10px 12px; }
    .source, .result {
      overflow-y: auto;
      white-space: pre-wrap;
      word-break: break-word;
    }
    .source {
      max-height: 120px;
      color: #9a9aa5;
      font-size: 13px;
      margin-bottom: 8px;
      padding-bottom: 8px;
      border-bottom: 1px dashed #3a3a44;
    }
    .result { max-height: 240px; }
    .loading { color: #7ab3ff; }
    .error { color: #ff7a7a; }
    .footer {
      display: flex;
      justify-content: flex-end;
      padding: 8px 12px;
      border-top: 1px solid #3a3a44;
    }
    .copy {
      background: #3a3a44;
      color: #e8e8ec;
      border-radius: 6px;
      padding: 4px 10px;
      font-size: 13px;
    }
    .copy:hover { background: #4a4a56; }
  `;
  const PANEL_WIDTH = 400;

  let host = null;
  // Request currently shown in the panel; results of older requests are dropped
  let current = null;

  function removePanel() {
    if (host) {
      host.remove();
      host = null;
      current = null;
    }
  }

  // Selected text with line breaks intact, including inside text fields
  function selectedText() {
    const el = document.activeElement;
    if (el && (el.tagName === "TEXTAREA" || el.tagName === "INPUT")) {
      // null for input types without a text selection (email, number, …)
      if (typeof el.selectionStart === "number" && el.selectionEnd > el.selectionStart) {
        return el.value.slice(el.selectionStart, el.selectionEnd).trim();
      }
    }
    const sel = window.getSelection();
    return sel ? sel.toString().trim() : "";
  }

  function element(tag, className, text) {
    const el = document.createElement(tag);
    el.className = className;
    if (text != null) el.textContent = text;
    return el;
  }

  async function copyText(text) {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch (e) {
      // the async clipboard API needs a secure (https) page; fall back to execCommand
    }
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.style.cssText = "position:fixed;top:0;left:0;opacity:0;";
    document.documentElement.appendChild(ta);
    ta.select();
    let ok = false;
    try {
      ok = document.execCommand("copy");
    } catch (e) {
      ok = false;
    }
    ta.remove();
    return ok;
  }

  function showPanel(id, text) {
    removePanel();
    const sel = window.getSelection();
    let rect = null;
    if (sel && sel.rangeCount > 0) {
      rect = sel.getRangeAt(0).getBoundingClientRect();
      if (!rect.width && !rect.height) rect = null;
    }

    // A custom element name keeps generic page selectors (div, span, …) off the host
    host = document.createElement("qwen-translate-panel");
    const root = host.attachShadow({ mode: "closed" });
    const style = document.createElement("style");
    style.textContent = STYLE;

    const panel = element("div", "panel");
    const header = element("div", "header");
    const closeBtn = element("button", "close", "×");
    closeBtn.title = "关闭";
    header.append(element("span", "title", "Qwen 本地翻译"), closeBtn);

    const body = element("div", "body");
    const result = element("div", "result loading", "翻译中…");
    body.append(element("div", "source", text), result);

    const footer = element("div", "footer");
    const copyBtn = element("button", "copy", "复制译文");
    footer.append(copyBtn);

    panel.append(header, body, footer);
    root.append(style, panel);

    const viewWidth = document.documentElement.clientWidth;
    const width = Math.min(PANEL_WIDTH, viewWidth * 0.9);
    let x = rect ? rect.left + window.scrollX : window.scrollX + 40;
    const y = rect ? rect.bottom + window.scrollY + 8 : window.scrollY + 40;
    x = Math.min(x, window.scrollX + viewWidth - width - 12);
    host.style.setProperty("--qt-left", `${Math.max(4, x)}px`);
    host.style.setProperty("--qt-top", `${Math.max(4, y)}px`);
    document.documentElement.appendChild(host);

    current = { id, result };
    closeBtn.addEventListener("click", removePanel);
    copyBtn.addEventListener("click", async () => {
      const ok = await copyText(result.textContent);
      copyBtn.textContent = ok ? "已复制" : "复制失败";
      setTimeout(() => (copyBtn.textContent = "复制译文"), 1500);
    });
  }

  function showResult(msg) {
    const { result } = current;
    result.classList.remove("loading");
    if (msg.translation != null) {
      result.textContent = msg.translation;
    } else {
      result.textContent = "翻译失败: " + (msg.error || "未知错误");
      result.classList.add("error");
    }
  }

  api.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
    if (msg.type === "qt-show") {
      const text = selectedText() || msg.text;
      showPanel(msg.id, text);
      sendResponse({ text });
    } else if (msg.type === "qt-result" && current && msg.id === current.id) {
      showResult(msg);
    }
  });

  // Clicks inside the shadow root are retargeted to the host element
  document.addEventListener("mousedown", (e) => {
    if (host && !e.composedPath().includes(host)) removePanel();
  });
  document.addEventListener("keydown", (e) => {
    if (host && e.key === "Escape") removePanel();
  });
})();
