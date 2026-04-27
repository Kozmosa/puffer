//! Thin WebSocket client for the `puffer daemon` wire protocol.
//!
//! Protocol (one JSON object per WebSocket text frame):
//!   server → client on connect:
//!     { event: "hello", payload: { protocolVersion, workspaceRoot } }
//!   client → server:
//!     { id, method, params }
//!   server → client (correlated by id):
//!     { id, result } | { id, error: { code, message } }
//!   server → client (fire-and-forget streaming):
//!     { event: "<channel>", payload }
//!
//! One DaemonClient manages one connection. Callers:
//!   client.request("list_grouped_sessions", {})  // → Promise<result>
//!   client.on("session:<sid>:event", handler)    // subscribe to an event
//!
//! The first consumer who needs the daemon calls ensureDaemonClient(), which
//! either asks Tauri to start a local daemon or (when remote) uses the
//! supplied URL + token.

import { invoke } from "@tauri-apps/api/core";

export type DaemonHandshake = {
  url: string;
  token: string;
  protocolVersion: string;
  workspaceRoot: string;
};

type Pending = {
  resolve: (value: unknown) => void;
  reject: (reason: unknown) => void;
  timer: ReturnType<typeof setTimeout>;
};

type RpcError = { code: string; message: string };

export type ConnectionState = "idle" | "connecting" | "open" | "reconnecting" | "closed";
const DEFAULT_RPC_TIMEOUT_MS = 30_000;

export class DaemonClient {
  private ws: WebSocket | null = null;
  private pending = new Map<string, Pending>();
  private listeners = new Map<string, Set<(payload: unknown) => void>>();
  private connectionListeners = new Set<(state: ConnectionState) => void>();
  private nextId = 1;
  private readyPromise: Promise<void> | null = null;
  private _state: ConnectionState = "idle";
  private autoReconnect = true;
  private reconnectAttempt = 0;

  constructor(public readonly handshake: DaemonHandshake) {}

  get state(): ConnectionState {
    return this._state;
  }

  private setState(next: ConnectionState) {
    if (this._state === next) return;
    this._state = next;
    for (const fn of this.connectionListeners) fn(next);
  }

  /** Subscribe to connection-state changes ("connecting" | "open" |
   *  "reconnecting" | "closed"). UI shows a banner when this isn't "open". */
  onConnectionChange(handler: (state: ConnectionState) => void): () => void {
    this.connectionListeners.add(handler);
    // Fire the current state immediately so the caller doesn't race the
    // first real change.
    handler(this._state);
    return () => {
      this.connectionListeners.delete(handler);
    };
  }

  async connect(): Promise<void> {
    if (this.readyPromise) return this.readyPromise;
    this.setState("connecting");
    this.readyPromise = new Promise<void>((resolve, reject) => {
      const url = appendToken(this.handshake.url, this.handshake.token);
      const ws = new WebSocket(url);
      this.ws = ws;
      let opened = false;
      ws.addEventListener("open", () => {
        opened = true;
        this.reconnectAttempt = 0;
        this.setState("open");
      });
      ws.addEventListener("message", (event) => {
        this.dispatch(event.data);
      });
      ws.addEventListener("error", () => {
        if (!opened) reject(new Error(`daemon websocket failed: ${url}`));
      });
      ws.addEventListener("close", (ev) => {
        this.ws = null;
        const err = new Error(`daemon websocket closed (${ev.code})`);
        for (const [, pending] of this.pending) {
          clearTimeout(pending.timer);
          pending.reject(err);
        }
        this.pending.clear();
        if (!opened) {
          reject(err);
          this.setState("closed");
          return;
        }
        // Already opened at least once — surface the disconnect + kick off
        // an auto-reconnect loop. Listeners need to re-subscribe after a
        // successful reconnect; they can observe state transitions.
        if (this.autoReconnect) {
          this.setState("reconnecting");
          this.scheduleReconnect();
        } else {
          this.setState("closed");
        }
      });

      // First expected message is the "hello" event; treat that as ready.
      const helloHandler = (payload: unknown) => {
        this.off("hello", helloHandler);
        resolve();
      };
      this.on("hello", helloHandler);
    });
    return this.readyPromise;
  }

  private scheduleReconnect() {
    this.reconnectAttempt += 1;
    // Exponential backoff capped at 10s: 500 / 1000 / 2000 / 4000 / 8000 /
    // 10000…
    const delay = Math.min(500 * 2 ** (this.reconnectAttempt - 1), 10_000);
    setTimeout(() => {
      if (!this.autoReconnect) return;
      this.readyPromise = null;
      void this.connect().catch(() => {
        // connect() sets state to closed on first-open failure; schedule
        // another attempt so the caller doesn't have to.
        if (this.autoReconnect) this.scheduleReconnect();
      });
    }, delay);
  }

  private dispatch(raw: unknown) {
    if (typeof raw !== "string") return;
    let parsed: unknown;
    try {
      parsed = JSON.parse(raw);
    } catch {
      console.warn("daemon: non-JSON frame", raw);
      return;
    }
    if (!parsed || typeof parsed !== "object") return;
    const msg = parsed as { id?: string; result?: unknown; error?: RpcError; event?: string; payload?: unknown };
    if (msg.id !== undefined) {
      const pending = this.pending.get(msg.id);
      if (pending) {
        this.pending.delete(msg.id);
        clearTimeout(pending.timer);
        if (msg.error) {
          pending.reject(new Error(`${msg.error.code}: ${msg.error.message}`));
        } else {
          pending.resolve(msg.result);
        }
      }
      return;
    }
    if (msg.event !== undefined) {
      const set = this.listeners.get(msg.event);
      if (set) {
        for (const fn of set) fn(msg.payload);
      }
    }
  }

  /** Issues an RPC and resolves with the `result` field. */
  async request<T = unknown>(method: string, params: Record<string, unknown> = {}): Promise<T> {
    await this.connect();
    const ws = this.ws;
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      throw new Error("daemon websocket is not open");
    }
    const id = String(this.nextId++);
    const frame = JSON.stringify({ id, method, params });
    return new Promise<T>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`daemon RPC timed out: ${method}`));
      }, DEFAULT_RPC_TIMEOUT_MS);
      this.pending.set(id, {
        resolve: (value) => resolve(value as T),
        reject,
        timer
      });
      ws.send(frame);
    });
  }

  /** Subscribe to a server-sent event channel. Returns a disposer. */
  on<T = unknown>(event: string, handler: (payload: T) => void): () => void {
    let set = this.listeners.get(event);
    if (!set) {
      set = new Set();
      this.listeners.set(event, set);
    }
    set.add(handler as (payload: unknown) => void);
    return () => this.off(event, handler);
  }

  off<T = unknown>(event: string, handler: (payload: T) => void): void {
    const set = this.listeners.get(event);
    if (!set) return;
    set.delete(handler as (payload: unknown) => void);
    if (set.size === 0) this.listeners.delete(event);
  }

  close() {
    this.autoReconnect = false;
    this.ws?.close();
    this.ws = null;
    this.setState("closed");
  }
}

const BROWSER_HANDSHAKE_STORAGE_KEY = "puffer-desktop:daemon-handshake";

function appendToken(url: string, token: string): string {
  try {
    const parsed = new URL(url);
    if (!parsed.searchParams.has("token")) {
      parsed.searchParams.set("token", token);
    }
    return parsed.toString();
  } catch {
    return url.includes("?")
      ? `${url}&token=${encodeURIComponent(token)}`
      : `${url}?token=${encodeURIComponent(token)}`;
  }
}

// ---------------------------------------------------------------------------
// Singleton management — local (via Tauri) and remote (paste URL+token).
// ---------------------------------------------------------------------------

let sharedClient: DaemonClient | null = null;
let sharedConnectPromise: Promise<DaemonClient> | null = null;

export function canInvokeTauri(): boolean {
  if (typeof window === "undefined") return false;
  const tauriWindow = window as unknown as {
    __TAURI_INTERNALS__?: unknown;
    __TAURI__?: unknown;
  };
  return Boolean(tauriWindow.__TAURI_INTERNALS__) || Boolean(tauriWindow.__TAURI__);
}

function envValue(name: string): string | null {
  const env = (import.meta as unknown as { env?: Record<string, string | undefined> }).env ?? {};
  const value = env[name]?.trim();
  return value ? value : null;
}

function normalizeDaemonUrl(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) return trimmed;
  try {
    const parsed = new URL(trimmed);
    if (parsed.protocol === "http:") parsed.protocol = "ws:";
    if (parsed.protocol === "https:") parsed.protocol = "wss:";
    if (parsed.pathname === "/" || parsed.pathname === "") {
      parsed.pathname = "/ws";
    }
    return parsed.toString();
  } catch {
    return trimmed;
  }
}

function handshakeFromStorage(): DaemonHandshake | null {
  if (typeof window === "undefined") return null;
  const raw = window.localStorage.getItem(BROWSER_HANDSHAKE_STORAGE_KEY);
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as Partial<DaemonHandshake>;
    if (typeof parsed.url !== "string" || typeof parsed.token !== "string") return null;
    return {
      url: normalizeDaemonUrl(parsed.url),
      token: parsed.token,
      protocolVersion: parsed.protocolVersion ?? "1",
      workspaceRoot: parsed.workspaceRoot ?? ""
    };
  } catch {
    return null;
  }
}

function persistBrowserHandshake(handshake: DaemonHandshake): void {
  if (typeof window === "undefined") return;
  window.localStorage.setItem(BROWSER_HANDSHAKE_STORAGE_KEY, JSON.stringify(handshake));
}

function handshakeFromUrl(): DaemonHandshake | null {
  if (typeof window === "undefined") return null;
  const params = new URLSearchParams(window.location.search);
  const url =
    params.get("daemonUrl") ??
    params.get("pufferDaemonUrl") ??
    params.get("wsUrl") ??
    params.get("url");
  const token =
    params.get("daemonToken") ??
    params.get("pufferDaemonToken") ??
    params.get("token");
  if (!url || !token) return null;
  return {
    url: normalizeDaemonUrl(url),
    token,
    protocolVersion: params.get("daemonProtocolVersion") ?? "1",
    workspaceRoot: params.get("daemonWorkspaceRoot") ?? ""
  };
}

function handshakeFromEnv(): DaemonHandshake | null {
  const url = envValue("VITE_PUFFER_DAEMON_URL");
  const token = envValue("VITE_PUFFER_DAEMON_TOKEN");
  if (!url || !token) return null;
  return {
    url: normalizeDaemonUrl(url),
    token,
    protocolVersion: envValue("VITE_PUFFER_DAEMON_PROTOCOL_VERSION") ?? "1",
    workspaceRoot: envValue("VITE_PUFFER_DAEMON_WORKSPACE_ROOT") ?? ""
  };
}

/** Returns the browser-supplied daemon handshake, if one is configured.
 *  URL params win and are persisted so reloads do not require pasting the
 *  token again. */
export function configuredBrowserDaemonHandshake(): DaemonHandshake | null {
  const fromUrl = handshakeFromUrl();
  if (fromUrl) {
    persistBrowserHandshake(fromUrl);
    return fromUrl;
  }
  return handshakeFromEnv() ?? handshakeFromStorage();
}

/** Whether the renderer has any route to a daemon. Tauri can spawn one;
 *  browser mode needs a pre-supplied WebSocket handshake. */
export function canReachDaemon(): boolean {
  return canInvokeTauri() || configuredBrowserDaemonHandshake() !== null || sharedClient !== null;
}

/** Returns the singleton local daemon client, starting the subprocess if
 *  this is the first caller. In a browser this attaches to a configured
 *  WebSocket daemon instead of spawning a subprocess. */
export async function ensureLocalDaemonClient(): Promise<DaemonClient> {
  if (sharedClient) return sharedClient;
  if (sharedConnectPromise) return sharedConnectPromise;
  sharedConnectPromise = (async () => {
    const handshake = canInvokeTauri()
      ? await invoke<DaemonHandshake>("start_local_daemon")
      : configuredBrowserDaemonHandshake();
    if (!handshake) {
      throw new Error(
        "No Puffer daemon WebSocket configured. Start `puffer daemon --bind 127.0.0.1:1421 --token <token>` and open the app with `?daemonUrl=ws://127.0.0.1:1421/ws&daemonToken=<token>`."
      );
    }
    const client = new DaemonClient(handshake);
    await client.connect();
    sharedClient = client;
    return client;
  })();
  try {
    return await sharedConnectPromise;
  } finally {
    sharedConnectPromise = null;
  }
}

/** Returns (and caches) a client against a remote daemon's URL + token.
 *  Each distinct URL caches its own client so switching remotes works. */
const remoteClients = new Map<string, Promise<DaemonClient>>();
export async function ensureRemoteDaemonClient(
  url: string,
  token: string
): Promise<DaemonClient> {
  const key = `${url}\x00${token}`;
  const existing = remoteClients.get(key);
  if (existing) return existing;
  const promise = (async () => {
    const handshake: DaemonHandshake = {
      url,
      token,
      protocolVersion: "1",
      workspaceRoot: ""
    };
    const client = new DaemonClient(handshake);
    await client.connect();
    return client;
  })();
  remoteClients.set(key, promise);
  return promise;
}

/** Swaps the shared daemon client — used when the user connects to a remote
 *  daemon. Existing connections are closed; pending subscribers need to
 *  re-subscribe after the swap. Returns the new live client. */
export async function switchDaemonClient(handshake: DaemonHandshake): Promise<DaemonClient> {
  // Tear down the old client first so listeners / RPCs on it surface as
  // "closed" errors rather than silently dropping frames.
  if (sharedClient) {
    try {
      sharedClient.close();
    } catch {
      /* ignore */
    }
    sharedClient = null;
  }
  sharedConnectPromise = null;
  const client = new DaemonClient(handshake);
  await client.connect();
  sharedClient = client;
  return client;
}

/** Returns the currently-shared client if one is open, without attempting
 *  to start a new daemon. Useful for UI that wants to know the active
 *  workspace without forcing a spawn. */
export function currentDaemonClient(): DaemonClient | null {
  return sharedClient;
}
