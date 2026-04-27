<script lang="ts" module>
  export type IconName =
    | "file"
    | "folder"
    | "folderOpen"
    | "edit"
    | "terminal"
    | "play"
    | "pause"
    | "pause2"
    | "plus"
    | "sparkles"
    | "arrow"
    | "arrowUp"
    | "chevR"
    | "chevD"
    | "chevL"
    | "git"
    | "branch"
    | "globe"
    | "check"
    | "x"
    | "settings"
    | "test"
    | "search"
    | "plug"
    | "bolt"
    | "bug"
    | "repo"
    | "clock"
    | "cpu"
    | "moreH"
    | "eye"
    | "panel"
    | "layers"
    | "refresh"
    | "flame"
    | "rocket"
    | "key"
    | "lock"
    | "shield"
    | "server"
    | "link"
    | "logs"
    | "external"
    | "coin"
    | "token"
    | "wrench"
    | "pin";
</script>

<script lang="ts">
  type Props = {
    name: IconName;
    size?: number;
    color?: string;
    strokeWidth?: number;
  };

  let { name, size = 14, color, strokeWidth = 1.8 }: Props = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke={color ?? "currentColor"}
  stroke-width={strokeWidth}
  stroke-linecap="round"
  stroke-linejoin="round"
>
  {#if name === "file"}
    <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
    <polyline points="14 2 14 8 20 8" />
  {:else if name === "folder"}
    <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
  {:else if name === "folderOpen"}
    <path d="M6 14l1.45-2.9A2 2 0 0 1 9.24 10H20a2 2 0 0 1 1.94 2.5l-1.55 6a2 2 0 0 1-1.94 1.5H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.93a2 2 0 0 1 1.66.9l.82 1.2a2 2 0 0 0 1.66.9H18a2 2 0 0 1 2 2v2" />
  {:else if name === "edit"}
    <path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7" />
    <path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z" />
  {:else if name === "terminal"}
    <polyline points="4 17 10 11 4 5" />
    <line x1="12" y1="19" x2="20" y2="19" />
  {:else if name === "play"}
    <polygon points="5 3 19 12 5 21 5 3" />
  {:else if name === "pause"}
    <rect x="6" y="4" width="4" height="16" />
    <rect x="14" y="4" width="4" height="16" />
  {:else if name === "pause2"}
    <rect x="6" y="4" width="4" height="16" rx="1" />
    <rect x="14" y="4" width="4" height="16" rx="1" />
  {:else if name === "plus"}
    <path d="M12 5v14" />
    <path d="M5 12h14" />
  {:else if name === "sparkles"}
    <path d="M12 3l1.5 5L18 9.5 13.5 11 12 16l-1.5-5L6 9.5 10.5 8z" />
    <path d="M19 14l.7 2.3L22 17l-2.3.7L19 20l-.7-2.3L16 17l2.3-.7z" />
  {:else if name === "arrow"}
    <path d="M5 12h14" />
    <polyline points="12 5 19 12 12 19" />
  {:else if name === "arrowUp"}
    <line x1="12" y1="19" x2="12" y2="5" />
    <polyline points="5 12 12 5 19 12" />
  {:else if name === "chevR"}
    <polyline points="9 18 15 12 9 6" />
  {:else if name === "chevD"}
    <polyline points="6 9 12 15 18 9" />
  {:else if name === "chevL"}
    <polyline points="15 18 9 12 15 6" />
  {:else if name === "git"}
    <circle cx="6" cy="6" r="3" />
    <circle cx="6" cy="18" r="3" />
    <circle cx="18" cy="12" r="3" />
    <path d="M9 6h6a3 3 0 0 1 3 3v3" />
    <path d="M6 9v6" />
  {:else if name === "branch"}
    <line x1="6" y1="3" x2="6" y2="15" />
    <circle cx="18" cy="6" r="3" />
    <circle cx="6" cy="18" r="3" />
    <path d="M18 9a9 9 0 0 1-9 9" />
  {:else if name === "globe"}
    <circle cx="12" cy="12" r="10" />
    <line x1="2" y1="12" x2="22" y2="12" />
    <path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z" />
  {:else if name === "check"}
    <polyline points="20 6 9 17 4 12" />
  {:else if name === "x"}
    <path d="M18 6 6 18" />
    <path d="m6 6 12 12" />
  {:else if name === "settings"}
    <circle cx="12" cy="12" r="3" />
    <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
  {:else if name === "test"}
    <path d="M9 2v6l-5 9a4 4 0 0 0 4 5h8a4 4 0 0 0 4-5l-5-9V2" />
    <line x1="9" y1="2" x2="15" y2="2" />
  {:else if name === "search"}
    <circle cx="11" cy="11" r="8" />
    <path d="M21 21l-4.3-4.3" />
  {:else if name === "plug"}
    <path d="M9 2v6" />
    <path d="M15 2v6" />
    <path d="M6 8h12v4a4 4 0 0 1-4 4h-4a4 4 0 0 1-4-4z" />
    <path d="M12 16v6" />
  {:else if name === "bolt"}
    <polyline points="13 2 3 14 12 14 11 22 21 10 12 10 13 2" />
  {:else if name === "bug"}
    <rect x="8" y="6" width="8" height="14" rx="4" />
    <path d="M19 7l-3 2" />
    <path d="M5 7l3 2" />
    <path d="M19 13h-3" />
    <path d="M5 13h3" />
    <path d="M19 19l-3-2" />
    <path d="M5 19l3-2" />
  {:else if name === "repo"}
    <path d="M3 6a3 3 0 0 1 3-3h12a2 2 0 0 1 2 2v15a2 2 0 0 1-2 2H6a3 3 0 0 1-3-3z" />
    <path d="M3 6h17" />
    <path d="M3 18h17" />
  {:else if name === "clock"}
    <circle cx="12" cy="12" r="10" />
    <polyline points="12 6 12 12 16 14" />
  {:else if name === "cpu"}
    <rect x="4" y="4" width="16" height="16" rx="2" />
    <rect x="9" y="9" width="6" height="6" />
    <line x1="9" y1="2" x2="9" y2="4" />
    <line x1="15" y1="2" x2="15" y2="4" />
    <line x1="9" y1="20" x2="9" y2="22" />
    <line x1="15" y1="20" x2="15" y2="22" />
    <line x1="20" y1="9" x2="22" y2="9" />
    <line x1="20" y1="14" x2="22" y2="14" />
    <line x1="2" y1="9" x2="4" y2="9" />
    <line x1="2" y1="14" x2="4" y2="14" />
  {:else if name === "moreH"}
    <circle cx="12" cy="12" r="1" />
    <circle cx="19" cy="12" r="1" />
    <circle cx="5" cy="12" r="1" />
  {:else if name === "eye"}
    <path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z" />
    <circle cx="12" cy="12" r="3" />
  {:else if name === "panel"}
    <rect x="3" y="3" width="18" height="18" rx="2" />
    <line x1="9" y1="3" x2="9" y2="21" />
  {:else if name === "layers"}
    <polygon points="12 2 2 7 12 12 22 7 12 2" />
    <polyline points="2 17 12 22 22 17" />
    <polyline points="2 12 12 17 22 12" />
  {:else if name === "refresh"}
    <polyline points="23 4 23 10 17 10" />
    <polyline points="1 20 1 14 7 14" />
    <path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10" />
    <path d="M20.49 15a9 9 0 0 1-14.85 3.36L1 14" />
  {:else if name === "flame"}
    <path d="M8 14s-2-2-2-5 2-5 4-5c1 0 2 .5 2 2 0 0 0-3 3-3 4 0 5 4 5 7 0 5-4 9-7 9s-5-2-5-5z" />
  {:else if name === "rocket"}
    <path d="M4.5 16.5c-1.5 1.26-2 5-2 5s3.74-.5 5-2c.71-.84.7-2.13-.09-2.91a2.18 2.18 0 0 0-2.91-.09z" />
    <path d="M12 15l-3-3a22 22 0 0 1 2-3.95A12.88 12.88 0 0 1 22 2c0 2.72-.78 7.5-6 11a22.35 22.35 0 0 1-4 2z" />
    <path d="M9 12H4s.55-3.03 2-4c1.62-1.08 5 0 5 0" />
    <path d="M12 15v5s3.03-.55 4-2c1.08-1.62 0-5 0-5" />
  {:else if name === "key"}
    <circle cx="8" cy="15" r="4" />
    <path d="M10.85 12.15 19 4" />
    <path d="M18 5l3 3" />
    <path d="M15 8l2 2" />
  {:else if name === "lock"}
    <rect x="3" y="11" width="18" height="10" rx="2" />
    <path d="M7 11V7a5 5 0 0 1 10 0v4" />
  {:else if name === "shield"}
    <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
  {:else if name === "server"}
    <rect x="2" y="3" width="20" height="7" rx="1" />
    <rect x="2" y="14" width="20" height="7" rx="1" />
    <line x1="6" y1="6.5" x2="6.01" y2="6.5" />
    <line x1="6" y1="17.5" x2="6.01" y2="17.5" />
  {:else if name === "link"}
    <path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71" />
    <path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71" />
  {:else if name === "logs"}
    <path d="M4 6h16" />
    <path d="M4 12h16" />
    <path d="M4 18h10" />
  {:else if name === "external"}
    <path d="M15 3h6v6" />
    <path d="M10 14 21 3" />
    <path d="M21 14v5a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5" />
  {:else if name === "coin"}
    <line x1="12" y1="1" x2="12" y2="23" />
    <path d="M17 5H9.5a3.5 3.5 0 0 0 0 7h5a3.5 3.5 0 0 1 0 7H6" />
  {:else if name === "token"}
    <circle cx="12" cy="12" r="9" />
    <circle cx="12" cy="12" r="4" />
  {:else if name === "wrench"}
    <path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z" />
  {:else if name === "pin"}
    <path d="M12 17v5" />
    <path d="M5 17h14" />
    <path d="M7 3h10l-2 7 3 4H6l3-4z" />
  {/if}
</svg>
