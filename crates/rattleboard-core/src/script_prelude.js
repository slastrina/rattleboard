// Rattleboard script prelude, version 1.
//
// Every engine (Rust/QuickJS on the desktop and Rattleboard Web, QuickJS on Android,
// JavaScriptCore on Apple platforms) evaluates exactly this file as a classic script in a fresh
// context, before the plugin's module, after defining two globals:
//
//   __rbHost   an object with three native functions, strings in and strings out:
//              fetch(id, requestJson) -> '{"ok":true}' or '{"error":"code","message":"..."}'
//              hmac(algo, keyJson, data) -> '{"hex":"..."}' or '{"error":"code","message":"..."}'
//              log(line) -> undefined
//   __rbInput  the run's input as a JSON string (see the porting spec, section "Input")
//
// The script's completion value is the runner: { start(fn), settle(id, payloadJson), poll() }.
// The host never trusts anything this file does: hosts, secrets, limits and the result are all
// checked again on the host side. This file only gives scripts a friendly API.
(function () {
  "use strict";
  const g = globalThis;
  const host = g.__rbHost;
  const input = JSON.parse(g.__rbInput);
  delete g.__rbHost;
  delete g.__rbInput;

  const stringify = JSON.stringify;
  const parse = JSON.parse;
  const freeze = Object.freeze;
  const defineProperty = Object.defineProperty;
  const keysOf = Object.keys;
  const hasOwn = (o, k) => Object.prototype.hasOwnProperty.call(o, k);
  const RealDate = Date;
  const RealPromise = Promise;
  const RealMap = Map;
  const RealWeakMap = WeakMap;

  // ------------------------------------------------------------ no code from strings

  const noEval = function () {
    throw new EvalError("Code generation from strings isn't allowed in Rattleboard scripts");
  };
  delete g.eval;
  for (const f of [function () {}, async function () {}, function* () {}, async function* () {}]) {
    defineProperty(Object.getPrototypeOf(f), "constructor", { value: noEval, writable: false, enumerable: false, configurable: false });
  }
  noEval.prototype = Function.prototype;
  defineProperty(g, "Function", { value: noEval, writable: false, enumerable: false, configurable: false });
  // Atomics.wait could block the thread past every limit; nothing here shares memory anyway.
  delete g.Atomics;
  delete g.SharedArrayBuffer;

  // ------------------------------------------------------------ the clock

  // Date.now() and new Date() are the run's time (ctx.now); it doesn't advance during a run.
  const NOW = input.now;
  function RbDate(...a) {
    if (new.target === undefined) return new RealDate(NOW).toString();
    return a.length === 0 ? new RealDate(NOW) : new RealDate(...a);
  }
  RbDate.prototype = RealDate.prototype;
  RbDate.now = () => NOW;
  RbDate.parse = RealDate.parse;
  RbDate.UTC = RealDate.UTC;
  defineProperty(RealDate.prototype, "constructor", { value: RbDate, writable: true, enumerable: false, configurable: true });
  defineProperty(g, "Date", { value: RbDate, writable: true, enumerable: false, configurable: true });

  // ------------------------------------------------------------ log

  const show = (v) => {
    if (typeof v === "string") return v;
    try {
      const s = stringify(v);
      return s === undefined ? String(v) : s;
    } catch (e) {
      return String(v);
    }
  };
  const log = (...a) => {
    host.log(a.map(show).join(" "));
  };
  defineProperty(g, "console", { value: freeze({ log, info: log, warn: log, error: log, debug: log }), writable: true, enumerable: false, configurable: true });

  // ------------------------------------------------------------ secrets

  const declared = input.secrets;
  const isSet = input.secretsSet;
  const tokens = new RealWeakMap();
  const tokenProto = freeze({
    toString() {
      return `{{secret.${tokens.get(this)}}}`;
    },
    toJSON() {
      return `{{secret.${tokens.get(this)}}}`;
    },
    [Symbol.toPrimitive]() {
      return `{{secret.${tokens.get(this)}}}`;
    },
  });
  function secret(name) {
    name = String(name);
    if (!declared.includes(name)) throw new TypeError(`No secret called ${name} is declared`);
    const t = Object.create(tokenProto);
    tokens.set(t, name);
    return freeze(t);
  }
  function hasSecret(name) {
    return isSet.includes(String(name));
  }

  function hmac(algo, key, data) {
    const k = tokens.has(key) ? { secret: tokens.get(key) } : { text: String(key) };
    const r = parse(host.hmac(String(algo), stringify(k), String(data)));
    if (r.error) throw new TypeError(r.message);
    return r.hex;
  }

  // ------------------------------------------------------------ fetch

  class FetchError extends Error {
    constructor(code, message) {
      super(message);
      this.name = "FetchError";
      this.code = code;
    }
  }
  let seq = 0;
  const pending = new RealMap();

  function fetch(url, init) {
    try {
      if (init === undefined || init === null) init = {};
      if (typeof init !== "object") throw new FetchError("bad_request", "fetch(url, init): init must be an object");
      const headers = {};
      if (init.headers !== undefined && init.headers !== null) {
        if (typeof init.headers !== "object") throw new FetchError("bad_request", "fetch(url, init): headers must be an object");
        for (const k of keysOf(init.headers)) headers[k] = String(init.headers[k]);
      }
      let body = init.body;
      if (body === undefined || body === null) body = null;
      else if (typeof body !== "string") {
        body = stringify(body);
        if (body === undefined) throw new FetchError("bad_request", "fetch(url, init): body must be a string or JSON data");
        if (!keysOf(headers).some((k) => k.toLowerCase() === "content-type")) headers["content-type"] = "application/json";
      }
      const method = init.method === undefined || init.method === null ? "GET" : String(init.method);
      const id = ++seq;
      const r = parse(host.fetch(id, stringify({ url: String(url), method, headers, body })));
      if (r.error) throw new FetchError(r.error, r.message);
      return new RealPromise((resolve, reject) => pending.set(id, { resolve, reject }));
    } catch (e) {
      return RealPromise.reject(e);
    }
  }

  function response(r) {
    const body = r.body;
    return freeze({
      status: r.status,
      ok: r.status >= 200 && r.status < 300,
      url: r.url,
      headers: freeze(r.headers),
      text: () => RealPromise.resolve(body),
      json: () => {
        try {
          return RealPromise.resolve(parse(body));
        } catch (e) {
          return RealPromise.reject(new SyntaxError("The response isn't valid JSON"));
        }
      },
    });
  }

  // ------------------------------------------------------------ store

  const MAX_STORE = 16384;
  const utf8len = (s) => {
    let n = 0;
    for (let i = 0; i < s.length; i++) {
      const c = s.charCodeAt(i);
      if (c < 0x80) n += 1;
      else if (c < 0x800) n += 2;
      else if (c >= 0xd800 && c < 0xdc00 && i + 1 < s.length) {
        n += 4;
        i++;
      } else n += 3;
    }
    return n;
  };
  const data = parse(stringify(input.store));
  const store = freeze({
    get(key) {
      key = String(key);
      return hasOwn(data, key) ? parse(stringify(data[key])) : undefined;
    },
    set(key, value) {
      key = String(key);
      if (key.length === 0 || key.length > 100) throw new RangeError("store keys are 1 to 100 characters");
      if (value === undefined) {
        delete data[key];
        return;
      }
      const s = stringify(value);
      if (s === undefined) throw new TypeError("store.set: the value must be JSON data");
      const before = hasOwn(data, key) ? data[key] : undefined;
      data[key] = parse(s);
      if (utf8len(stringify(data)) > MAX_STORE) {
        if (before === undefined) delete data[key];
        else data[key] = before;
        throw new RangeError("The store is full (16 KB at most)");
      }
    },
    delete(key) {
      delete data[String(key)];
    },
    keys() {
      return keysOf(data);
    },
  });

  // ------------------------------------------------------------ ctx

  const deepFreeze = (v) => {
    if (v && typeof v === "object") {
      for (const k of keysOf(v)) deepFreeze(v[k]);
      freeze(v);
    }
    return v;
  };
  const ctx = freeze({
    settings: deepFreeze(input.settings),
    board: freeze({ rows: input.board.rows, cols: input.board.cols }),
    now: NOW,
    local: input.local,
    utcOffset: input.utcOffset,
    name: input.name,
    fetch,
    secret,
    hasSecret,
    hmac,
    log,
    store,
  });

  // ------------------------------------------------------------ the runner

  let outcome = null;
  const errorOf = (e) => {
    if (e !== null && typeof e === "object" && typeof e.message === "string") {
      return { name: typeof e.name === "string" ? e.name : "Error", message: e.message, code: typeof e.code === "string" ? e.code : null };
    }
    return { name: "Error", message: show(e), code: null };
  };
  return {
    start(fn) {
      if (typeof fn !== "function") return "nofn";
      RealPromise.resolve()
        .then(() => fn(ctx))
        .then(
          (v) => {
            outcome = { value: v };
          },
          (e) => {
            outcome = { error: errorOf(e) };
          },
        );
      return "";
    },
    settle(id, payload) {
      const p = pending.get(id);
      if (!p) return;
      pending.delete(id);
      const r = parse(payload);
      if (r.error) p.reject(new FetchError(r.error, r.message));
      else p.resolve(response(r));
    },
    poll() {
      if (outcome === null) return "";
      if (outcome.error) return stringify({ error: outcome.error });
      let value;
      try {
        value = outcome.value === undefined ? "null" : stringify(outcome.value);
      } catch (e) {
        return stringify({ invalid: "The result isn't plain data (JSON): " + errorOf(e).message });
      }
      if (value === undefined) value = "null";
      return '{"value":' + value + ',"store":' + stringify(data) + "}";
    },
  };
})();
