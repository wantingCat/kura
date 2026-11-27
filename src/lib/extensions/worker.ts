/// <reference lib="webworker" />

import * as cheerio from "cheerio";
import CryptoJSOrig from "crypto-js";
import { transform } from "sucrase";
import type { ExtensionFetchReq, ExtensionFetchResp } from "../api";
import {
  compareTitles,
  extractPartNumber,
  extractSeasonNumber,
  extractYear,
  normalizeTitle,
  parseEpisodeNumber,
  sanitizeQuery,
} from "../titleMatch";

// ---------------------------------------------------------------------------
// Capture Trusted Worker Primitives & Lock Down Ambient Browser APIs (`C-01`)
// ---------------------------------------------------------------------------

const workerPostMessage = self.postMessage.bind(self);
const workerSetTimeout = self.setTimeout.bind(self);

function lockdownAmbientGlobals() {
  const blocked = [
    "XMLHttpRequest",
    "WebSocket",
    "EventSource",
    "Worker",
    "SharedWorker",
    "importScripts",
    "indexedDB",
    "IDBFactory",
    "IDBDatabase",
    "IDBKeyRange",
    "caches",
    "CacheStorage",
    "BroadcastChannel",
    "WebTransport",
    "RTCPeerConnection",
    "RTCDataChannel",
    "postMessage",
    "MessageChannel",
    "MessagePort",
    "FontFace",
    "FontFaceSet",
  ];

  // Scrub prototype chain first so inherited getters/methods (postMessage, indexedDB, caches, importScripts) cannot be reached
  let proto: unknown = Object.getPrototypeOf(self);
  while (proto && proto !== Object.prototype) {
    const rec = proto as Record<string, unknown>;
    for (const key of [...blocked, "fetch"]) {
      try {
        delete rec[key];
      } catch {
        try {
          Object.defineProperty(rec, key, {
            value: undefined,
            writable: false,
            configurable: false,
          });
        } catch {
          // ignore non-configurable host prototype slots
        }
      }
    }
    proto = Object.getPrototypeOf(proto);
  }

  const g = globalThis as Record<string, unknown>;
  for (const key of blocked) {
    try {
      delete g[key];
    } catch {
      // ignore
    }
    try {
      Object.defineProperty(g, key, {
        value: undefined,
        writable: false,
        configurable: false,
      });
    } catch {
      g[key] = undefined;
    }
  }

  const navBlocked = ["sendBeacon", "storage", "locks", "credentials", "serviceWorker"];
  if (typeof self.navigator === "object" && self.navigator !== null) {
    let navObj: unknown = self.navigator;
    while (navObj && navObj !== Object.prototype) {
      const rec = navObj as Record<string, unknown>;
      for (const key of navBlocked) {
        try {
          delete rec[key];
        } catch {
          // ignore
        }
        try {
          Object.defineProperty(rec, key, {
            value: undefined,
            writable: false,
            configurable: false,
          });
        } catch {
          // ignore non-configurable host slot
        }
      }
      navObj = Object.getPrototypeOf(navObj);
    }
  }
}

lockdownAmbientGlobals();

// ---------------------------------------------------------------------------
// Worker <-> Main Thread RPC Protocol
// ---------------------------------------------------------------------------

interface InitPayload {
  id: string;
  language: string;
  payload: string;
  userPrefs: Record<string, string>;
}

type IncomingMsg =
  | { kind: "init"; callId: number; data: InitPayload }
  | { kind: "update_prefs"; prefs: Record<string, string> }
  | { kind: "call"; callId: number; method: string; args: unknown[] }
  | { kind: "http_fetch_done"; reqId: number; ok: boolean; resp?: ExtensionFetchResp; error?: string };

let nextFetchId = 1;
const pendingFetches = new Map<
  number,
  { resolve: (r: ExtensionFetchResp) => void; reject: (e: Error) => void }
>();

let userPreferences: Record<string, string> = {};
// eslint-disable-next-line @typescript-eslint/no-explicit-any
let ProviderCtor: (new () => Record<string, (...args: any[]) => any>) | null = null;

// ---------------------------------------------------------------------------
// Binary / Encoding Helpers (`Buffer`, `$toBytes`, `$toString`)
// ---------------------------------------------------------------------------

const textEncoder = new TextEncoder();
const textDecoder = new TextDecoder();

function bytesToBase64(bytes: Uint8Array): string {
  let binary = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(binary);
}

function base64ToBytes(b64: string): Uint8Array {
  if (!b64) return new Uint8Array(0);
  const normalized = b64.replace(/-/g, "+").replace(/_/g, "/").replace(/\s+/g, "");
  if (!normalized) return new Uint8Array(0);
  const padded = normalized + "=".repeat((4 - (normalized.length % 4)) % 4);
  const bin = atob(padded);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) {
    out[i] = bin.charCodeAt(i);
  }
  return out;
}

function bytesToHex(bytes: Uint8Array): string {
  let out = "";
  for (let i = 0; i < bytes.length; i++) {
    out += bytes[i].toString(16).padStart(2, "0");
  }
  return out;
}

function hexToBytes(hex: string): Uint8Array {
  const clean = hex.trim().replace(/^0x/i, "");
  const out = new Uint8Array(Math.floor(clean.length / 2));
  for (let i = 0; i < out.length; i++) {
    out[i] = parseInt(clean.slice(i * 2, i * 2 + 2), 16) || 0;
  }
  return out;
}

class KuraBuffer extends Uint8Array {
  static poolSize = 8192;

  static override from(
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    input: any,
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    encodingOrMapFn?: any,
  ): KuraBuffer {
    if (typeof encodingOrMapFn === "function") {
      return new KuraBuffer(Uint8Array.from(input, encodingOrMapFn));
    }
    const encoding = typeof encodingOrMapFn === "string" ? encodingOrMapFn : undefined;
    if (typeof input === "string") {
      const enc = (encoding ?? "utf8").toLowerCase();
      if (enc === "base64" || enc === "base64url") {
        return new KuraBuffer(base64ToBytes(input));
      }
      if (enc === "hex") {
        return new KuraBuffer(hexToBytes(input));
      }
      if (enc === "latin1" || enc === "binary" || enc === "ascii") {
        const arr = new KuraBuffer(input.length);
        for (let i = 0; i < input.length; i++) arr[i] = input.charCodeAt(i) & 0xff;
        return arr;
      }
      return new KuraBuffer(textEncoder.encode(input));
    }
    if (input instanceof ArrayBuffer) {
      return new KuraBuffer(new Uint8Array(input));
    }
    if (ArrayBuffer.isView(input)) {
      return new KuraBuffer(new Uint8Array(input.buffer, input.byteOffset, input.byteLength));
    }
    if (Array.isArray(input) || (input && typeof input.length === "number")) {
      return new KuraBuffer(Uint8Array.from(input));
    }
    return new KuraBuffer(0);
  }

  static alloc(size: number, fill?: string | number, encoding?: string): KuraBuffer {
    const buf = new KuraBuffer(Math.max(0, size | 0));
    if (typeof fill === "number") {
      buf.fill(fill & 0xff);
    } else if (typeof fill === "string" && fill.length > 0) {
      const src = KuraBuffer.from(fill, encoding);
      if (src.length > 0) {
        for (let i = 0; i < buf.length; i++) buf[i] = src[i % src.length];
      }
    }
    return buf;
  }

  equals(other: Uint8Array): boolean {
    if (!other || this.length !== other.length) return false;
    for (let i = 0; i < this.length; i++) {
      if (this[i] !== other[i]) return false;
    }
    return true;
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  override toString(encoding?: any): string {
    const enc = String(encoding ?? "utf8").toLowerCase();
    if (enc === "base64") return bytesToBase64(this);
    if (enc === "base64url") {
      return bytesToBase64(this).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
    }
    if (enc === "hex") return bytesToHex(this);
    if (enc === "latin1" || enc === "binary") {
      let s = "";
      for (let i = 0; i < this.length; i++) s += String.fromCharCode(this[i]);
      return s;
    }
    return textDecoder.decode(this);
  }
}

// ---------------------------------------------------------------------------
// CryptoJS Wrapper (supports both WordArray and Uint8Array keys/IVs)
// ---------------------------------------------------------------------------

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function normalizeCryptoArg(val: any): any {
  if (val instanceof Uint8Array) {
    return CryptoJSOrig.lib.WordArray.create(val as unknown as number[]);
  }
  if (val instanceof ArrayBuffer) {
    return CryptoJSOrig.lib.WordArray.create(new Uint8Array(val) as unknown as number[]);
  }
  return val;
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function normalizeCryptoCfg(cfg: any): any {
  if (!cfg || typeof cfg !== "object") return cfg;
  const copy = { ...cfg };
  if (copy.iv) copy.iv = normalizeCryptoArg(copy.iv);
  return copy;
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function wrapEncoder(enc: any) {
  return {
    ...enc,
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    stringify: (input: any) => enc.stringify(normalizeCryptoArg(input)),
    parse: (input: string) => enc.parse(input),
  };
}

const KuraCryptoJS = {
  ...CryptoJSOrig,
  AES: {
    ...CryptoJSOrig.AES,
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    encrypt: (message: any, key: any, cfg?: any) =>
      CryptoJSOrig.AES.encrypt(
        typeof message === "string" ? message : normalizeCryptoArg(message),
        typeof key === "string" ? key : normalizeCryptoArg(key),
        normalizeCryptoCfg(cfg),
      ),
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    decrypt: (message: any, key: any, cfg?: any) =>
      CryptoJSOrig.AES.decrypt(
        message,
        typeof key === "string" ? key : normalizeCryptoArg(key),
        normalizeCryptoCfg(cfg),
      ),
  },
  enc: {
    ...CryptoJSOrig.enc,
    Utf8: wrapEncoder(CryptoJSOrig.enc.Utf8),
    Base64: wrapEncoder(CryptoJSOrig.enc.Base64),
    Hex: wrapEncoder(CryptoJSOrig.enc.Hex),
    Latin1: wrapEncoder(CryptoJSOrig.enc.Latin1),
    Utf16: wrapEncoder(CryptoJSOrig.enc.Utf16),
    Utf16LE: wrapEncoder(CryptoJSOrig.enc.Utf16LE),
  },
};

// ---------------------------------------------------------------------------
// Seanime `LoadDoc` & `Doc` (`cheerio` -> `DocSelection` contract)
// ---------------------------------------------------------------------------

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function wrapDocSelection($: cheerio.CheerioAPI, sel: cheerio.Cheerio<any>): any {
  const api = {
    attr(name: string): string | undefined {
      return sel.attr(name);
    },
    attrs(): Record<string, string> {
      const first = sel.get(0);
      if (first && "attribs" in first && first.attribs) {
        return { ...first.attribs };
      }
      return {};
    },
    children(selector?: string) {
      return wrapDocSelection($, selector ? sel.children(selector) : sel.children());
    },
    closest(selector?: string) {
      return wrapDocSelection($, selector ? sel.closest(selector) : sel.closest("*"));
    },
    contents() {
      return wrapDocSelection($, sel.contents());
    },
    contentsFiltered(selector: string) {
      return wrapDocSelection($, sel.contents().filter(selector));
    },
    data(name?: string) {
      if (typeof name === "string") {
        const v = sel.data(name);
        return v === undefined ? undefined : String(v);
      }
      const d = sel.data();
      const out: Record<string, string> = {};
      if (d && typeof d === "object") {
        for (const [k, v] of Object.entries(d)) out[k] = String(v);
      }
      return out;
    },
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    each(cb: (index: number, element: any) => void) {
      sel.each((idx, el) => {
        cb(idx, wrapDocSelection($, $(el)));
      });
      return api;
    },
    end() {
      return wrapDocSelection($, sel.end());
    },
    eq(index: number) {
      return wrapDocSelection($, sel.eq(index));
    },
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    filter(selector: string | ((index: number, element: any) => boolean)) {
      if (typeof selector === "function") {
        return wrapDocSelection(
          $,
          sel.filter((idx, el) => Boolean(selector(idx, wrapDocSelection($, $(el))))),
        );
      }
      return wrapDocSelection($, sel.filter(selector));
    },
    find(selector: string) {
      return wrapDocSelection($, sel.find(selector));
    },
    first() {
      return wrapDocSelection($, sel.first());
    },
    has(selector: string) {
      return wrapDocSelection($, sel.has(selector));
    },
    text(): string {
      return sel.text();
    },
    html(): string | null {
      return sel.html();
    },
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    is(selector: string | ((index: number, element: any) => boolean)): boolean {
      if (typeof selector === "function") {
        return sel.is((idx, el) => Boolean(selector(idx, wrapDocSelection($, $(el)))));
      }
      return sel.is(selector);
    },
    last() {
      return wrapDocSelection($, sel.last());
    },
    length(): number {
      return sel.length;
    },
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    map<T>(cb: (index: number, element: any) => T): T[] {
      const results: T[] = [];
      sel.each((idx, el) => {
        results.push(cb(idx, wrapDocSelection($, $(el))));
      });
      return results;
    },
    next(selector?: string) {
      return wrapDocSelection($, selector ? sel.next(selector) : sel.next());
    },
    nextAll(selector?: string) {
      return wrapDocSelection($, selector ? sel.nextAll(selector) : sel.nextAll());
    },
    nextUntil(selector: string, filter?: string) {
      return wrapDocSelection($, sel.nextUntil(selector, filter));
    },
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    not(selector: string | ((index: number, element: any) => boolean)) {
      if (typeof selector === "function") {
        return wrapDocSelection(
          $,
          sel.not((idx, el) => Boolean(selector(idx, wrapDocSelection($, $(el))))),
        );
      }
      return wrapDocSelection($, sel.not(selector));
    },
    parent(selector?: string) {
      return wrapDocSelection($, selector ? sel.parent(selector) : sel.parent());
    },
    parents(selector?: string) {
      return wrapDocSelection($, selector ? sel.parents(selector) : sel.parents());
    },
    parentsUntil(selector: string, filter?: string) {
      return wrapDocSelection($, sel.parentsUntil(selector, filter));
    },
    prev(selector?: string) {
      return wrapDocSelection($, selector ? sel.prev(selector) : sel.prev());
    },
    prevAll(selector?: string) {
      return wrapDocSelection($, selector ? sel.prevAll(selector) : sel.prevAll());
    },
    prevUntil(selector: string, filter?: string) {
      return wrapDocSelection($, sel.prevUntil(selector, filter));
    },
    siblings(selector?: string) {
      return wrapDocSelection($, selector ? sel.siblings(selector) : sel.siblings());
    },
  };
  return api;
}

function LoadDoc(html: string) {
  const $ = cheerio.load(html ?? "");
  return (selector: string) => wrapDocSelection($, $(selector));
}

class Doc {
  constructor(html: string) {
    const $ = cheerio.load(html ?? "");
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    return wrapDocSelection($, $.root()) as any;
  }
}

// ---------------------------------------------------------------------------
// Sandboxed `fetch` Shim (`res.text()`, `res.json()`, `res.bytes()` lazy!)
// ---------------------------------------------------------------------------

function makeCaseInsensitiveMap<V>(raw: Record<string, V>): Record<string, V> {
  const lowerMap: Record<string, V> = {};
  for (const [k, v] of Object.entries(raw ?? {})) {
    lowerMap[k.toLowerCase()] = v;
  }
  return new Proxy(lowerMap, {
    get(target, prop) {
      if (typeof prop === "string") {
        return target[prop.toLowerCase()];
      }
      return Reflect.get(target, prop);
    },
    has(target, prop) {
      if (typeof prop === "string") {
        return prop.toLowerCase() in target;
      }
      return Reflect.has(target, prop);
    },
  });
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
async function seanimeFetch(url: string, options?: any) {
  const reqId = nextFetchId++;
  let bodyStr: string | undefined;
  let bodyBase64: string | undefined;

  if (options?.body !== undefined && options?.body !== null) {
    const b = options.body;
    if (typeof b === "string") {
      bodyStr = b;
    } else if (b instanceof URLSearchParams) {
      bodyStr = b.toString();
    } else if (b instanceof Uint8Array) {
      bodyBase64 = bytesToBase64(b);
    } else if (ArrayBuffer.isView(b)) {
      bodyBase64 = bytesToBase64(new Uint8Array(b.buffer, b.byteOffset, b.byteLength));
    } else if (b instanceof ArrayBuffer) {
      bodyBase64 = bytesToBase64(new Uint8Array(b));
    } else if (typeof b === "object") {
      bodyStr = JSON.stringify(b);
    } else {
      bodyStr = String(b);
    }
  }

  const headers: Record<string, string> = {};
  if (options?.headers && typeof options.headers === "object") {
    if (typeof Headers !== "undefined" && options.headers instanceof Headers) {
      options.headers.forEach((v: string, k: string) => {
        headers[k] = String(v);
      });
    } else if (Array.isArray(options.headers)) {
      for (const pair of options.headers) {
        if (Array.isArray(pair) && pair.length >= 2 && pair[1] !== undefined && pair[1] !== null) {
          headers[String(pair[0])] = String(pair[1]);
        }
      }
    } else {
      for (const [k, v] of Object.entries(options.headers)) {
        if (v !== undefined && v !== null) headers[k] = String(v);
      }
    }
  }

  const req: ExtensionFetchReq = {
    url: String(url),
    method: options?.method ? String(options.method) : "GET",
    headers,
    body: bodyStr,
    bodyBase64,
    redirect: options?.redirect ?? "follow",
    timeout: typeof options?.timeout === "number" ? options.timeout : 35,
  };

  const resp = await new Promise<ExtensionFetchResp>((resolve, reject) => {
    pendingFetches.set(reqId, { resolve, reject });
    workerPostMessage({ kind: "http_fetch", reqId, req });
  });

  const ciHeaders = makeCaseInsensitiveMap(resp.headers);
  const ciRawHeaders = makeCaseInsensitiveMap(resp.rawHeaders);

  // Backend populates ONLY `bodyText` (valid UTF-8) OR `bodyBase64` (binary) — decode/encode lazily!
  let cachedBytes: Uint8Array | null = null;
  let cachedText: string | null = null;

  function getBytes(): Uint8Array {
    if (cachedBytes) return cachedBytes;
    if (resp.bodyBase64) {
      cachedBytes = base64ToBytes(resp.bodyBase64);
    } else if (resp.bodyText) {
      cachedBytes = textEncoder.encode(resp.bodyText);
    } else {
      cachedBytes = new Uint8Array(0);
    }
    return cachedBytes;
  }

  function getText(): string {
    if (cachedText !== null) return cachedText;
    if (resp.bodyText || !resp.bodyBase64) {
      cachedText = resp.bodyText ?? "";
    } else {
      cachedText = textDecoder.decode(getBytes());
    }
    return cachedText;
  }

  return {
    status: resp.status,
    statusText: resp.statusText,
    method: resp.method,
    rawHeaders: ciRawHeaders,
    ok: resp.ok,
    url: resp.url,
    headers: ciHeaders,
    cookies: resp.cookies ?? {},
    redirected: resp.redirected,
    contentType: resp.contentType,
    contentLength: resp.contentLength,
    text(): string {
      return getText();
    },
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    json<T = any>(): T {
      return JSON.parse(getText()) as T;
    },
    bytes(): Uint8Array {
      return getBytes();
    },
  };
}

// ---------------------------------------------------------------------------
// `$store`, `$shared`, `$habari`, `$goFeed`, `$scannerUtils`, `$torrentUtils`
// ---------------------------------------------------------------------------

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function deepClone<T>(v: T): T {
  if (v === null || typeof v !== "object") return v;
  try {
    return structuredClone(v);
  } catch {
    return JSON.parse(JSON.stringify(v));
  }
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const storeData = new Map<string, any>();
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const storeWatchers = new Map<string, ((v: any) => void)[]>();

const $store = {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  set(key: string, value: any) {
    storeData.set(key, deepClone(value));
    const listeners = storeWatchers.get(key);
    if (listeners) for (const cb of listeners) cb(deepClone(value));
  },
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  get<T = any>(key: string): T {
    return deepClone(storeData.get(key));
  },
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  getUnsafe<T = any>(key: string): T {
    return storeData.get(key);
  },
  has(key: string): boolean {
    return storeData.has(key);
  },
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  getOrSet<T = any>(key: string, setFunc: () => T): T {
    if (storeData.has(key)) return deepClone(storeData.get(key));
    const val = setFunc();
    storeData.set(key, deepClone(val));
    return val;
  },
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  setIfLessThanLimit<T = any>(key: string, value: T, maxAllowedElements: number): boolean {
    if (storeData.size >= maxAllowedElements && !storeData.has(key)) return false;
    storeData.set(key, deepClone(value));
    return true;
  },
  unmarshalJSON(data: string) {
    const obj = JSON.parse(data);
    if (obj && typeof obj === "object") {
      for (const [k, v] of Object.entries(obj)) storeData.set(k, v);
    }
  },
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  marshalJSON(value: any): string {
    return JSON.stringify(value);
  },
  reset() {
    storeData.clear();
  },
  values() {
    return Array.from(storeData.values()).map((v) => deepClone(v));
  },
  valuesUnsafe() {
    return Array.from(storeData.values());
  },
  getAll() {
    return deepClone(Object.fromEntries(storeData.entries()));
  },
  getAllUnsafe() {
    return Object.fromEntries(storeData.entries());
  },
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  watch<T = any>(key: string, callback: (value: T) => void) {
    const arr = storeWatchers.get(key) ?? [];
    arr.push(callback);
    storeWatchers.set(key, arr);
  },
};

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const sharedFactories = new Map<string, () => any>();
const $shared = {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  define<T = any>(name: string, factory: () => T) {
    sharedFactories.set(name, factory);
  },
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  use<T = any>(name: string): T {
    const fn = sharedFactories.get(name);
    if (!fn) throw new Error(`$shared helper '${name}' is not registered`);
    return fn();
  },
};

const $habari = {
  parse(filename: string) {
    const raw = String(filename ?? "").trim();
    const extMatch = raw.match(/\.([a-zA-Z0-9]{2,4})$/);
    const file_extension = extMatch ? extMatch[1] : undefined;
    const withoutExt = extMatch ? raw.slice(0, -extMatch[0].length) : raw;
    const groupMatch = withoutExt.match(/^\[([^\]]+)\]/);
    const release_group = groupMatch ? groupMatch[1].trim() : undefined;
    const resMatch = withoutExt.match(/\b(2160p|4k|1440p|1080p|720p|576p|540p|480p)\b/i);
    const video_resolution = resMatch ? resMatch[1] : undefined;
    const crcMatch = withoutExt.match(/\[([0-9A-Fa-f]{8})\]/);
    const file_checksum = crcMatch ? crcMatch[1] : undefined;

    const seasonNum = extractSeasonNumber(withoutExt);
    const season_number = seasonNum > 0 ? [String(seasonNum)] : undefined;
    const partNum = extractPartNumber(withoutExt);
    const part_number = partNum > 0 ? [String(partNum)] : undefined;
    const parsedEp = parseEpisodeNumber(withoutExt);
    const episode_number = parsedEp !== null ? [String(parsedEp)] : undefined;

    const cleanTitle = withoutExt
      .replace(/^\[[^\]]+\]\s*/, "")
      .replace(/\[[^\]]*\]|\([^)]*\)/g, " ")
      .replace(/-\s*\d{1,4}(?:v\d)?(?=$|[\s\[\(_.-]).*$/, "")
      .replace(/\bS\d{1,2}E\d{1,4}.*$/i, "")
      .replace(/[._]+/g, " ")
      .replace(/\s+/g, " ")
      .trim();

    return {
      file_name: raw,
      file_extension,
      file_checksum,
      release_group,
      video_resolution,
      season_number,
      part_number,
      episode_number,
      title: cleanTitle || raw,
      formatted_title: cleanTitle || raw,
    };
  },
};

const $goFeed = {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  parse(xml: string): Record<string, any> {
    const $ = cheerio.load(xml ?? "", { xmlMode: true });
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const items: Record<string, any>[] = [];
    $("item, entry").each((_, el) => {
      const node = $(el);
      const title = node.find("title").first().text().trim();
      const link =
        node.find("link").first().text().trim() ||
        node.find("link").first().attr("href") ||
        "";
      const pubDate =
        node.find("pubDate").first().text().trim() ||
        node.find("published").first().text().trim() ||
        node.find("updated").first().text().trim();
      const guid =
        node.find("guid").first().text().trim() ||
        node.find("id").first().text().trim();
      const description =
        node.find("description").first().text().trim() ||
        node.find("summary").first().text().trim();

      const custom: Record<string, string> = {};
      const extensions: Record<string, Record<string, { value: string }[]>> = {};
      node.children().each((__, child) => {
        const tagName = "tagName" in child && child.tagName ? String(child.tagName) : "";
        if (!tagName) return;
        const val = $(child).text().trim();
        if (tagName.includes(":")) {
          const [ns, local] = tagName.split(":", 2);
          custom[local] = val;
          custom[tagName] = val;
          extensions[ns] ??= {};
          extensions[ns][local] ??= [];
          extensions[ns][local].push({ value: val });
        }
      });

      const enclosures: { url: string; length?: string; type?: string }[] = [];
      node.find("enclosure").each((__, enc) => {
        const u = $(enc).attr("url");
        if (u) {
          enclosures.push({
            url: u,
            length: $(enc).attr("length"),
            type: $(enc).attr("type"),
          });
        }
      });

      items.push({
        title,
        link,
        links: link ? [link] : [],
        published: pubDate,
        pubDate,
        guid,
        description,
        enclosures,
        custom,
        extensions,
      });
    });

    return {
      title: $("channel > title, feed > title").first().text().trim(),
      items,
    };
  },
};

const $scannerUtils = {
  normalizeTitle,
  extractPartNumber,
  extractSeasonNumber,
  extractYear,
  compareTitles,
  findBestMatch(target: string, candidates: string[]): string {
    let best = "";
    let bestScore = -1;
    for (const c of candidates ?? []) {
      const s = compareTitles(target, c);
      if (s > bestScore) {
        bestScore = s;
        best = c;
      }
    }
    return best;
  },
  getSignificantTokens(title: string): string[] {
    return normalizeTitle(title).tokens;
  },
  buildSearchQuery(title: string): string {
    return normalizeTitle(title).cleanBaseTitle;
  },
  buildAdvancedQuery(titles: string[]): string {
    const uniq = Array.from(new Set((titles ?? []).map((t) => sanitizeQuery(t)).filter(Boolean)));
    return uniq.length <= 1 ? (uniq[0] ?? "") : `(${uniq.join(" | ")})`;
  },
  sanitizeQuery,
  buildSeasonQuery(title: string, season: number): string {
    const base = sanitizeQuery(title);
    if (!season || season <= 1) return base;
    const ord = season === 2 ? "2nd" : season === 3 ? "3rd" : `${season}th`;
    return `(${base} S0${season} | ${base} S${season} | ${base} Season ${season} | ${base} ${ord} Season)`;
  },
  buildPartQuery(title: string, part: number): string {
    const base = sanitizeQuery(title);
    if (!part || part <= 1) return base;
    return `(${base} Part ${part} | ${base} Cour ${part})`;
  },
  buildSmartSearchTitles(titles: string[]) {
    let season = -1;
    let part = -1;
    const set = new Set<string>();
    for (const raw of titles ?? []) {
      if (!raw) continue;
      const n = normalizeTitle(raw);
      if (season < 0 && n.season > 0) season = n.season;
      if (part < 0 && n.part > 0) part = n.part;
      if (n.cleanBaseTitle) set.add(n.cleanBaseTitle);
      if (raw.includes(":")) {
        const prefix = sanitizeQuery(raw.split(":")[0]);
        if (prefix.length >= 4) set.add(prefix);
      }
    }
    return { titles: Array.from(set), season, part };
  },
};

// ---------------------------------------------------------------------------
// Safe Bencode Root-Dict `4:info` Span Parser (`M-01`)
// ---------------------------------------------------------------------------

const MAX_BENCODE_DEPTH = 64;

function parseBencodeStringSpan(
  bytes: Uint8Array,
  pos: number,
): { dataStart: number; nextPos: number } | null {
  if (pos < 0 || pos >= bytes.length) return null;
  const first = bytes[pos];
  if (first < 0x30 || first > 0x39) return null;

  let colon = pos;
  let len = 0;
  while (colon < bytes.length && bytes[colon] !== 0x3a) {
    const d = bytes[colon];
    if (d < 0x30 || d > 0x39) return null;
    len = len * 10 + (d - 0x30);
    if (len > bytes.length) return null;
    colon++;
  }
  if (colon >= bytes.length || bytes[colon] !== 0x3a) return null;
  const dataStart = colon + 1;
  const nextPos = dataStart + len;
  if (nextPos > bytes.length) return null;
  return { dataStart, nextPos };
}

function skipBencode(bytes: Uint8Array, pos: number, depth = 0): number {
  if (depth > MAX_BENCODE_DEPTH || pos < 0 || pos >= bytes.length) return -1;
  const c = bytes[pos];
  if (c === 0x69) {
    // 'i' <int> 'e'
    const end = bytes.indexOf(0x65, pos + 1);
    if (end < 0) return -1;
    return end + 1;
  }
  if (c === 0x6c) {
    // 'l' <items> 'e'
    let cur = pos + 1;
    while (cur < bytes.length && bytes[cur] !== 0x65) {
      const next = skipBencode(bytes, cur, depth + 1);
      if (next <= cur) return -1;
      cur = next;
    }
    return cur < bytes.length && bytes[cur] === 0x65 ? cur + 1 : -1;
  }
  if (c === 0x64) {
    // 'd' <str-key><val> 'e'
    let cur = pos + 1;
    while (cur < bytes.length && bytes[cur] !== 0x65) {
      const keySpan = parseBencodeStringSpan(bytes, cur);
      if (!keySpan) return -1;
      const next = skipBencode(bytes, keySpan.nextPos, depth + 1);
      if (next <= keySpan.nextPos) return -1;
      cur = next;
    }
    return cur < bytes.length && bytes[cur] === 0x65 ? cur + 1 : -1;
  }
  const strSpan = parseBencodeStringSpan(bytes, pos);
  return strSpan ? strSpan.nextPos : -1;
}

const $torrentUtils = {
  getMagnetLinkFromTorrentData(b64: string): string {
    const bytes = base64ToBytes(b64);
    // Must start with a root bencode dictionary 'd'
    if (bytes.length < 2 || bytes[0] !== 0x64) return "";

    let cur = 1;
    let infoStart = -1;
    let infoEnd = -1;
    while (cur < bytes.length && bytes[cur] !== 0x65) {
      const keySpan = parseBencodeStringSpan(bytes, cur);
      if (!keySpan) return "";
      const keyLen = keySpan.nextPos - keySpan.dataStart;
      const isInfo =
        keyLen === 4 &&
        bytes[keySpan.dataStart] === 0x69 && // 'i'
        bytes[keySpan.dataStart + 1] === 0x6e && // 'n'
        bytes[keySpan.dataStart + 2] === 0x66 && // 'f'
        bytes[keySpan.dataStart + 3] === 0x6f; // 'o'
      const valStart = keySpan.nextPos;
      const valEnd = skipBencode(bytes, valStart, 1);
      if (valEnd <= valStart) return "";
      if (isInfo) {
        infoStart = valStart;
        infoEnd = valEnd;
        break;
      }
      cur = valEnd;
    }
    if (infoStart < 0 || infoEnd <= infoStart) return "";

    const infoSlice = bytes.subarray(infoStart, infoEnd);
    const wa = CryptoJSOrig.lib.WordArray.create(infoSlice as unknown as number[]);
    const hex = CryptoJSOrig.SHA1(wa).toString(CryptoJSOrig.enc.Hex);
    return `magnet:?xt=urn:btih:${hex}`;
  },
};

// ---------------------------------------------------------------------------
// Bind Seanime Globals onto `self` / `globalThis`
// ---------------------------------------------------------------------------

function installSeanimeGlobals() {
  const g = globalThis as Record<string, unknown>;
  g.__isOffline__ = false;
  try {
    Object.defineProperty(g, "fetch", {
      value: seanimeFetch,
      writable: false,
      configurable: false,
    });
  } catch {
    g.fetch = seanimeFetch;
  }
  g.LoadDoc = LoadDoc;
  g.Doc = Doc;
  g.CryptoJS = KuraCryptoJS;
  g.Buffer = KuraBuffer;
  g.$store = $store;
  g.$shared = $shared;
  g.$habari = $habari;
  g.$goFeed = $goFeed;
  g.$scannerUtils = $scannerUtils;
  g.$torrentUtils = $torrentUtils;
  g.$clone = deepClone;
  g.$toString = (v: unknown) => {
    if (v instanceof Uint8Array) return textDecoder.decode(v);
    if (typeof v === "string") return v;
    if (v === null || v === undefined) return "";
    if (typeof v === "object") return JSON.stringify(v);
    return String(v);
  };
  g.$toBytes = (v: unknown): Uint8Array => {
    if (v instanceof Uint8Array) return v;
    if (typeof v === "string") return textEncoder.encode(v);
    return textEncoder.encode(JSON.stringify(v ?? ""));
  };
  g.$sleep = (ms: number) => {
    return new Promise((r) => workerSetTimeout(r, Math.max(0, Number(ms) || 0)));
  };
  g.$getUserPreference = (key: string): string | undefined => {
    const v = userPreferences[key];
    return v === undefined || v === "" ? undefined : v;
  };
  g.$arrayOf = () => [];
  g.$replace = (target: Record<string, unknown>, next: Record<string, unknown>) => {
    if (target && next && typeof target === "object" && typeof next === "object") {
      for (const k of Object.keys(target)) delete target[k];
      Object.assign(target, next);
    }
  };
  g.$unmarshalJSON = (data: unknown, dst: Record<string, unknown>) => {
    const parsed = typeof data === "string" ? JSON.parse(data) : data;
    if (dst && parsed && typeof dst === "object" && typeof parsed === "object") {
      Object.assign(dst, parsed);
    }
  };
}

installSeanimeGlobals();

// ---------------------------------------------------------------------------
// Message Dispatcher
// ---------------------------------------------------------------------------

self.onmessage = async (ev: MessageEvent<IncomingMsg>) => {
  const msg = ev.data;
  if (!msg) return;

  if (msg.kind === "http_fetch_done") {
    const pending = pendingFetches.get(msg.reqId);
    if (!pending) return;
    pendingFetches.delete(msg.reqId);
    if (msg.ok && msg.resp) {
      pending.resolve(msg.resp);
    } else {
      pending.reject(new Error(msg.error ?? "Extension HTTP fetch failed"));
    }
    return;
  }

  if (msg.kind === "update_prefs") {
    userPreferences = msg.prefs ?? {};
    return;
  }

  if (msg.kind === "init") {
    try {
      lockdownAmbientGlobals();
      userPreferences = msg.data.userPrefs ?? {};
      let jsCode = msg.data.payload;
      if (msg.data.language.toLowerCase() === "typescript" || msg.data.language.toLowerCase() === "ts") {
        jsCode = transform(jsCode, { transforms: ["typescript"] }).code;
      }
      // Support CommonJS `exports.Provider` or top-level `class Provider`
      const exportsObj: Record<string, unknown> = {};
      const moduleObj = { exports: exportsObj };
      const compiled = new Function(
        "exports",
        "module",
        `${jsCode}\nreturn typeof Provider !== "undefined" ? Provider : (module.exports.Provider || module.exports.default || exports.Provider);`,
      );
      const ctor = compiled(exportsObj, moduleObj);
      if (typeof ctor !== "function") {
        throw new Error(`Extension '${msg.data.id}' did not define a Provider class`);
      }
      ProviderCtor = ctor;
      workerPostMessage({ kind: "result", callId: msg.callId, ok: true, value: true });
    } catch (e) {
      workerPostMessage({
        kind: "result",
        callId: msg.callId,
        ok: false,
        error: e instanceof Error ? e.message : String(e),
      });
    }
    return;
  }

  if (msg.kind === "call") {
    try {
      if (!ProviderCtor) {
        throw new Error("Extension worker is not initialized");
      }
      const instance = new ProviderCtor();
      const fn = instance[msg.method];
      if (typeof fn !== "function") {
        throw new Error(`Method '${msg.method}' is not defined on Provider`);
      }
      const rawResult = await fn.apply(instance, msg.args ?? []);
      // Clone to plain JSON-serializable data before posting across StructuredClone boundary
      const cleanResult = rawResult === undefined ? null : JSON.parse(JSON.stringify(rawResult));
      workerPostMessage({ kind: "result", callId: msg.callId, ok: true, value: cleanResult });
    } catch (e) {
      workerPostMessage({
        kind: "result",
        callId: msg.callId,
        ok: false,
        error: e instanceof Error ? e.message : String(e),
      });
    }
  }
};
