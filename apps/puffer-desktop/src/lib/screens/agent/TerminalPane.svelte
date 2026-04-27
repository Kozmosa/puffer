<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { Terminal } from "@xterm/xterm";
  import { FitAddon } from "@xterm/addon-fit";
  import "@xterm/xterm/css/xterm.css";
  import { ensureLocalDaemonClient } from "../../api/daemonClient";
  import { closePty, isDaemonReachable, openPty, resizePty, writePty } from "../../api/desktop";

  type Props = {
    /** Filesystem root the shell starts in. Sessions pass their cwd here. */
    cwd: string;
  };
  let { cwd }: Props = $props();

  let container: HTMLDivElement | null = $state(null);
  let term: Terminal | null = null;
  let fit: FitAddon | null = null;
  let ptyId: string | null = null;
  let disposers: Array<() => void> = [];
  let resizeObserver: ResizeObserver | null = null;
  let disposed = false;

  // Keep the active pty tied to this component instance; if the user
  // switches tabs away and back, Svelte re-mounts the component and we
  // get a fresh PTY (matches "one tab = one terminal" mental model).
  onMount(async () => {
    if (!container) return;
    const t = new Terminal({
      cursorBlink: true,
      fontFamily: '"JetBrains Mono", "JetBrainsMono Nerd Font", "SF Mono", Menlo, Consolas, monospace',
      fontSize: 13,
      letterSpacing: 0,
      theme: {
        background: "#ffffff",
        foreground: "#171717",
        cursor: "#171717",
        selectionBackground: "#d4d4d4",
        black: "#171717",
        red: "#b91c1c",
        green: "#15803d",
        yellow: "#a16207",
        blue: "#1d4ed8",
        magenta: "#9333ea",
        cyan: "#0e7490",
        white: "#f5f5f5",
        brightBlack: "#737373",
        brightRed: "#dc2626",
        brightGreen: "#16a34a",
        brightYellow: "#ca8a04",
        brightBlue: "#2563eb",
        brightMagenta: "#a855f7",
        brightCyan: "#0891b2",
        brightWhite: "#ffffff"
      }
    });
    const fa = new FitAddon();
    t.loadAddon(fa);
    t.open(container);
    try {
      fa.fit();
    } catch {
      /* container might be 0x0 on first paint; ResizeObserver will re-fit */
    }
    term = t;
    fit = fa;

    let cols = t.cols;
    let rows = t.rows;

    // Web preview: no daemon available, so render a friendly notice
    // instead of attempting the RPC and spamming a red error into the
    // terminal. The desktop (Tauri) build always has a daemon.
    if (!isDaemonReachable()) {
      t.writeln("\x1b[90mTerminal is available in the Puffer desktop app.\x1b[0m");
      t.writeln("\x1b[90mLaunch Puffer locally to get a live shell in this session's cwd.\x1b[0m");
      return;
    }

    try {
      const client = await ensureLocalDaemonClient();
      const { ptyId: id } = await openPty({ cwd, cols, rows });
      if (disposed) {
        // Component went away while we were awaiting; tear down the pty
        // we just opened so we don't leak a shell.
        await closePty(id).catch(() => {});
        return;
      }
      ptyId = id;

      disposers.push(
        client.on<{ data: string }>(`pty:${id}:data`, ({ data }) => {
          // Data arrives base64-encoded (the daemon doesn't assume UTF-8
          // for shell output). atob gives us a binary string; xterm's
          // `write` accepts that directly and handles the decoding.
          try {
            t.write(atob(data));
          } catch {
            /* malformed frame — skip */
          }
        })
      );
      disposers.push(
        client.on<{ exitCode: number }>(`pty:${id}:exit`, ({ exitCode }) => {
          t.writeln(`\r\n\x1b[90m[exit ${exitCode}]\x1b[0m`);
        })
      );

      const dataDisposable = t.onData((str) => {
        // btoa wants binary-string input; the UTF-8 dance below preserves
        // non-ASCII input (e.g. arrow-key sequences are ASCII anyway, but
        // paste of emoji etc. should roundtrip).
        const bytes = new TextEncoder().encode(str);
        let bin = "";
        for (const b of bytes) bin += String.fromCharCode(b);
        void writePty(id, btoa(bin)).catch(() => {});
      });
      disposers.push(() => dataDisposable.dispose());
    } catch (err) {
      t.writeln(`\r\n\x1b[31mterminal: ${String(err)}\x1b[0m`);
      return;
    }

    // Track container size so the PTY always matches the pane.
    const ro = new ResizeObserver(() => {
      if (!fit || !term || !ptyId) return;
      try {
        fit.fit();
      } catch {
        return;
      }
      const nextCols = term.cols;
      const nextRows = term.rows;
      if (nextCols !== cols || nextRows !== rows) {
        cols = nextCols;
        rows = nextRows;
        void resizePty(ptyId, cols, rows).catch(() => {});
      }
    });
    ro.observe(container);
    resizeObserver = ro;
  });

  onDestroy(() => {
    disposed = true;
    if (resizeObserver) {
      resizeObserver.disconnect();
      resizeObserver = null;
    }
    for (const d of disposers) {
      try {
        d();
      } catch {
        /* ignore */
      }
    }
    disposers = [];
    if (ptyId) {
      void closePty(ptyId).catch(() => {});
      ptyId = null;
    }
    if (term) {
      term.dispose();
      term = null;
    }
    fit = null;
  });
</script>

<div class="pf-terminal-pane">
  <div class="pf-terminal-host" bind:this={container}></div>
</div>

<style>
  .pf-terminal-pane {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--background);
  }
  .pf-terminal-host {
    flex: 1;
    min-height: 0;
    padding: 10px;
    background: var(--background);
  }
  /* xterm.js sets its own inline sizing; we just need the host to fill. */
  .pf-terminal-host :global(.xterm),
  .pf-terminal-host :global(.xterm-viewport),
  .pf-terminal-host :global(.xterm-screen) {
    height: 100%;
  }
  .pf-terminal-host :global(.xterm) {
    padding: 8px;
    border: 0;
    border-radius: 0;
    background: var(--background);
    letter-spacing: 0;
  }
  .pf-terminal-host :global(.xterm-viewport) {
    background: var(--background) !important;
  }
</style>
