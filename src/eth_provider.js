(function () {
  if (window.__wakuEth) return;
  var pending = new Map();
  var n = 1;
  function send(method, params) {
    return new Promise(function (resolve, reject) {
      if (!window.ipc || typeof window.ipc.postMessage !== "function") {
        var missing = new Error("Waku wallet bridge is not available");
        missing.code = 4100;
        reject(missing);
        return;
      }
      var id = n++;
      pending.set(id, { resolve: resolve, reject: reject });
      window.ipc.postMessage(
        JSON.stringify({ id: id, method: method, params: params || [] })
      );
    });
  }
  window.__wakuEthDone = function (msg) {
    var p = pending.get(msg.id);
    if (!p) return;
    pending.delete(msg.id);
    if (msg.ok) {
      p.resolve(msg.result);
      return;
    }
    var err = new Error((msg.error && msg.error.message) || "wallet error");
    err.code = msg.error && msg.error.code;
    p.reject(err);
  };
  var listeners = {};
  function on(ev, fn) {
    (listeners[ev] || (listeners[ev] = [])).push(fn);
  }
  function removeListener(ev, fn) {
    listeners[ev] = (listeners[ev] || []).filter(function (f) {
      return f !== fn;
    });
  }
  function emit(ev, payload) {
    (listeners[ev] || []).forEach(function (f) {
      try {
        f(payload);
      } catch (e) {}
    });
  }
  var provider = {
    isWaku: true,
    isProofShip: true,
    isMetaMask: false,
    request: function (args) {
      args = args || {};
      return send(args.method, args.params);
    },
    send: function (method, params) {
      if (method && typeof method === "object") {
        return send(method.method, method.params);
      }
      return send(method, params);
    },
    sendAsync: function (payload, cb) {
      send(payload.method, payload.params).then(
        function (result) {
          cb(null, { id: payload.id, jsonrpc: "2.0", result: result });
        },
        function (error) {
          cb(error);
        }
      );
    },
    on: on,
    addListener: on,
    removeListener: removeListener,
    emit: emit,
  };
  window.ethereum = provider;
  window.__wakuEth = provider;
  var info = Object.freeze({
    uuid: "a8e1d6c4-2b7f-4c19-9e5a-7f3b1d0c8e22",
    name: "ProofShip",
    rdns: "sh.waku.wallet",
    icon: "data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'><rect width='32' height='32' rx='8' fill='%230C1016'/><path d='M8 10 L18 16 L8 22 Z' fill='%233A4250'/><path d='M13 10 L24 16 L13 22 Z' fill='%23FF6B00'/></svg>",
  });
  function announce() {
    try {
      window.dispatchEvent(
        new CustomEvent("eip6963:announceProvider", {
          detail: Object.freeze({ info: info, provider: provider }),
        })
      );
    } catch (e) {}
  }
  window.addEventListener("eip6963:requestProvider", announce);
  announce();
})();
