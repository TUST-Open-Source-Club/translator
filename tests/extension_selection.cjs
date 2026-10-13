// Executes the real extension scripts using API/DOM doubles; NOT a browser test.
'use strict';
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const assert = require('node:assert/strict');
const repo = path.join(__dirname, '..');

async function scenario(build, apiName, frame, inaccessible, selection, expected, activeElement = null) {
  const nativeMessages = [], injections = [], panels = [];
  let receive, click;
  function el(tag) {
    return { tagName: tag.toUpperCase(), className: '', textContent: '', children: [],
      style: {setProperty() {}}, classList: {remove() {}, add() {}},
      append(...children) { this.children.push(...children); },
      attachShadow() { this.shadow = el('shadow'); return this.shadow; },
      addEventListener() {}, remove() {} };
  }
  const document = {activeElement, createElement: el, addEventListener() {},
    documentElement: {clientWidth: 1000, appendChild(host) { panels.push(host); }}};
  const api = {
    runtime: {onMessage: {addListener(fn) { receive = fn; }},
      onInstalled: {addListener() {}}, onStartup: {addListener() {}},
      connectNative() { return {onMessage: {addListener() {}}, onDisconnect: {addListener() {}},
        postMessage(msg) { nativeMessages.push(msg); }}; }},
    scripting: {async executeScript({target}) {
      injections.push(target.frameIds[0]);
      if (inaccessible && target.frameIds[0] !== 0) throw Error('Frame permission denied');
    }},
    tabs: {async sendMessage(_tab, msg) { return new Promise(resolve => receive(msg, {}, resolve)); }},
    contextMenus: {onClicked: {addListener(fn) { click = fn; }}},
  };
  const window = {getSelection: () => ({toString: () => selection, rangeCount: 0}), scrollX: 0, scrollY: 0};
  const context = vm.createContext({[apiName]: api, window, document, setTimeout, clearTimeout, navigator: {}});
  for (const file of ['content.js', 'background.js']) {
    vm.runInContext(fs.readFileSync(path.join(repo, build, file), 'utf8'), context);
  }
  await click({menuItemId: 'qt-translate-selection', frameId: frame, selectionText: 'Menu text'}, {id: 42});
  assert.deepEqual(injections, inaccessible ? [frame, 0] : [frame]);
  assert.equal(nativeMessages[0].text, expected);
  const source = panels[0].shadow.children[1].children[1].children[0];
  assert.equal(source.textContent, expected, 'panel source must match native request');
}

async function nativePortRecovery(build, apiName) {
  let click, connects = 0, posted = [];
  const api = {
    runtime: {
      onInstalled: {addListener() {}}, onStartup: {addListener() {}},
      connectNative() {
        const number = ++connects;
        const connection = {
          onMessage: {addListener(fn) { connection.message = fn; }},
          onDisconnect: {addListener(fn) { connection.disconnectHandler = fn; }},
          postMessage(msg) {
            if (number === 1) throw Error('host closed before send');
            posted.push(msg);
          },
          disconnect() {
            if (connection.disconnectHandler) connection.disconnectHandler({error: {message: 'closed'}});
          },
        };
        return connection;
      },
    },
    scripting: {async executeScript() {}},
    tabs: {sendMessage(_tab, msg) { return Promise.resolve({text: msg.text}); }},
    contextMenus: {removeAll() { return Promise.resolve(); }, create() {}, onClicked: {addListener(fn) { click = fn; }}},
  };
  const context = vm.createContext({[apiName]: api, setTimeout, clearTimeout});
  vm.runInContext(fs.readFileSync(path.join(repo, build, 'background.js'), 'utf8'), context);
  await click({menuItemId: 'qt-translate-selection', frameId: 0, selectionText: 'first'}, {id: 5});
  await click({menuItemId: 'qt-translate-selection', frameId: 0, selectionText: 'second'}, {id: 5});
  assert.equal(connects, 2, `${build}: reconnect after synchronous postMessage failure`);
  assert.deepEqual(JSON.parse(JSON.stringify(posted)), [{id: 2, text: 'second'}]);
}

(async () => {
  for (const [build, api] of [['chrome', 'chrome'], ['firefox', 'browser']]) {
    await scenario(build, api, 7, true, 'Old top-level selection', 'Menu text');
    await scenario(build, api, 7, false, 'Original\niframe selection', 'Original\niframe selection');
    await scenario(build, api, 0, false, 'Top-level\nselection', 'Top-level\nselection');
    await scenario(build, api, 0, false, '', 'Menu text');
    await scenario(build, api, 0, false, 'Other page text', 'Input\ntext',
      {tagName: 'TEXTAREA', value: 'Input\ntext', selectionStart: 0, selectionEnd: 10});
    await nativePortRecovery(build, api);
  }
  console.log('12 extension regressions passed (Node API/DOM doubles, not browser execution).');
})().catch(error => { console.error(error); process.exitCode = 1; });
