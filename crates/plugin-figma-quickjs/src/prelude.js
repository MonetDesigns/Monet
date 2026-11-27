// The `figma` global, built over a narrow bridge to the Rust side.
//
// This file runs once per realm, before the plugin's `main`. It creates the object the plugin
// sees, then removes the raw bridge from the global scope so plugin code cannot reach the host
// except through the API below.
//
// Deliberate shortcuts for M0 (see docs/figma-plugin-compat-plan.md): node proxies carry scalar
// properties only, ids are `hi:lo` strings, `figma.mixed` is a sentinel, and operations the engine
// has no command for throw a catchable `Error`.
(function () {
  'use strict';

  var bridge = globalThis.__figcraft;
  if (!bridge) {
    throw new Error('figcraft: the host bridge is missing');
  }
  delete globalThis.__figcraft;
  globalThis.__figcraft = undefined;

  var MIXED = Symbol('figma.mixed');
  var nodes = new Map();
  var pages = new Map();
  var handlers = Object.create(null);

  function fail(message) {
    throw new Error(String(message));
  }

  function parse(text) {
    return JSON.parse(text);
  }

  function encode(value) {
    if (value === undefined) {
      return 'null';
    }
    var text;
    try {
      text = JSON.stringify(value);
    } catch (error) {
      fail('the value is not JSON: ' + error.message);
    }
    return text === undefined ? 'null' : text;
  }

  function idOf(node) {
    if (typeof node === 'string') {
      return node;
    }
    if (node && typeof node === 'object' && typeof node.__id === 'string') {
      return node.__id;
    }
    fail('expected a node');
  }

  function fmt(value) {
    if (typeof value === 'string') {
      return value;
    }
    try {
      return typeof value === 'object' && value !== null ? JSON.stringify(value) : String(value);
    } catch (error) {
      return String(value);
    }
  }

  function log() {
    bridge.log(Array.prototype.map.call(arguments, fmt).join(' '));
  }

  // A node is a Proxy: reading any property asks the host, writing any property queues a
  // `node.set`, and a property the property table doesn't know throws from Rust instead of
  // silently becoming a JavaScript own property.
  function node(idText) {
    var cached = nodes.get(idText);
    if (cached) {
      return cached;
    }
    var target = {
      __id: idText,
      id: idText,
      remove: function () {
        bridge.remove(idText);
        nodes.delete(idText);
      },
      appendChild: function (child) {
        bridge.append(idOf(child), idText);
        return child;
      },
    };
    var proxy = new Proxy(target, {
      get: function (self, prop) {
        if (typeof prop !== 'string') {
          return undefined;
        }
        if (prop in self) {
          return self[prop];
        }
        return parse(bridge.get(idText, prop));
      },
      set: function (self, prop, value) {
        if (typeof prop !== 'string' || prop in self) {
          self[prop] = value;
          return true;
        }
        bridge.set(idText, prop, encode(value));
        return true;
      },
      has: function (self, prop) {
        return prop in self;
      },
    });
    nodes.set(idText, proxy);
    return proxy;
  }

  function page(idText) {
    var cached = pages.get(idText);
    if (cached) {
      return cached;
    }
    var target = { __id: idText, id: idText };
    var proxy = new Proxy(target, {
      get: function (self, prop) {
        if (typeof prop !== 'string') {
          return undefined;
        }
        if (prop === 'selection') {
          return parse(bridge.selection()).map(node);
        }
        if (prop in self) {
          return self[prop];
        }
        return parse(bridge.get(idText, prop));
      },
      set: function (self, prop, value) {
        if (prop === 'selection') {
          bridge.setSelection(encode(Array.prototype.map.call(value || [], idOf)));
          return true;
        }
        if (typeof prop !== 'string' || prop === 'id' || prop === '__id') {
          return false;
        }
        bridge.set(idText, prop, encode(value));
        return true;
      },
    });
    pages.set(idText, proxy);
    return proxy;
  }

  var figma = {
    get currentPage() { return page(bridge.page()); },
    get mixed() { return MIXED; },
    get apiVersion() { return '1.0.0'; },
    getNodeById: function (id) {
      return bridge.exists(String(id)) ? node(String(id)) : null;
    },
    createRectangle: function () { return node(bridge.create('rectangle')); },
    createEllipse: function () { return node(bridge.create('ellipse')); },
    createFrame: function () { return node(bridge.create('frame')); },
    createText: function () { return node(bridge.create('text')); },
    createLine: function () { return node(bridge.create('line')); },
    createVector: function () { return node(bridge.create('vector')); },
    createPolygon: function () { return node(bridge.create('polygon')); },
    createStar: function () { return node(bridge.create('star')); },
    createComponent: function () { return node(bridge.create('component')); },
    createSection: function () { return node(bridge.create('section')); },
    createSlice: function () { return node(bridge.create('slice')); },
    notify: function (message, options) {
      bridge.notify(String(message), options ? encode(options) : '');
    },
    closePlugin: function (message) {
      bridge.close(message === undefined ? '' : String(message));
    },
    commitUndo: function () { bridge.commitUndo(); },
    showUI: function (html, options) {
      bridge.showUi(String(html), options ? encode(options) : '');
    },
    ui: {
      onmessage: null,
      postMessage: function (message) { bridge.postMessage(encode(message)); },
      close: function () { bridge.closeUi(); },
    },
    on: function (type, handler) {
      (handlers[type] || (handlers[type] = [])).push(handler);
    },
    off: function (type, handler) {
      var list = handlers[type];
      if (!list) {
        return;
      }
      var at = list.indexOf(handler);
      if (at >= 0) {
        list.splice(at, 1);
      }
    },
    once: function (type, handler) {
      function once() {
        figma.off(type, once);
        handler.apply(null, arguments);
      }
      figma.on(type, once);
    },
  };

  globalThis.figma = figma;
  globalThis.console = { log: log, info: log, warn: log, error: log, debug: log };
  globalThis.__figma_dispatch = function (eventJson) {
    var event = JSON.parse(eventJson);
    if (event.type === 'message' && typeof figma.ui.onmessage === 'function') {
      figma.ui.onmessage(event.message);
    }
    var list = handlers[event.type];
    if (list) {
      list.slice().forEach(function (handler) { handler(event); });
    }
  };
})();
