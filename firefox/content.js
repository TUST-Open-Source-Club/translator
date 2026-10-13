"use strict";

(() => {
  if (window.__qwenTranslateInjected) return;
  window.__qwenTranslateInjected = true;

  let panel = null;
  // Request currently shown in the panel; results of older requests are dropped
  let currentId = null;

  function removePanel() {
    if (panel) {
      panel.remove();
      panel = null;
      currentId = null;
    }
  }

  function showPanel(id, text) {
    removePanel();
    currentId = id;
    const sel = window.getSelection();
    let rect = null;
    if (sel && sel.rangeCount > 0) {
      rect = sel.getRangeAt(0).getBoundingClientRect();
    }

    panel = document.createElement("div");
    panel.className = "qt-panel";

    const header = document.createElement("div");
    header.className = "qt-header";
    const title = document.createElement("span");
    title.textContent = "Qwen 本地翻译";
    const closeBtn = document.createElement("button");
    closeBtn.className = "qt-close";
    closeBtn.title = "关闭";
    closeBtn.textContent = "×";
    header.append(title, closeBtn);

    const body = document.createElement("div");
    body.className = "qt-body";
    const source = document.createElement("div");
    source.className = "qt-source";
    source.textContent = text;
    const result = document.createElement("div");
    result.className = "qt-result qt-loading";
    result.textContent = "翻译中…";
    body.append(source, result);

    const footer = document.createElement("div");
    footer.className = "qt-footer";
    const copyBtn = document.createElement("button");
    copyBtn.className = "qt-copy";
    copyBtn.textContent = "复制译文";
    footer.append(copyBtn);

    panel.append(header, body, footer);
    document.documentElement.appendChild(panel);

    const pw = 400;
    let x = rect ? rect.left + window.scrollX : window.scrollX + 40;
    let y = rect ? rect.bottom + window.scrollY + 8 : window.scrollY + 40;
    x = Math.min(x, window.scrollX + document.documentElement.clientWidth - pw - 12);
    panel.style.left = Math.max(4, x) + "px";
    panel.style.top = Math.max(4, y) + "px";

    closeBtn.addEventListener("click", removePanel);
    copyBtn.addEventListener("click", () => {
      navigator.clipboard.writeText(result.textContent).catch(() => {});
    });
  }

  browser.runtime.onMessage.addListener((msg) => {
    if (msg.type === "qt-show") {
      showPanel(msg.id, msg.text);
    } else if (msg.type === "qt-result" && panel && msg.id === currentId) {
      const result = panel.querySelector(".qt-result");
      result.classList.remove("qt-loading");
      if (msg.translation != null) {
        result.textContent = msg.translation;
      } else {
        result.textContent = "翻译失败: " + (msg.error || "未知错误");
        result.classList.add("qt-error");
      }
    }
  });

  document.addEventListener("mousedown", (e) => {
    if (panel && !panel.contains(e.target)) removePanel();
  });
})();
