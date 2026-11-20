# Local GPUI/Tauri observations

2026-11-11, running Stonks processes before this shell/composer parity pass.
Read `/proc/<pid>/smaps_rollup`; values are KiB, converted to MiB below.

| Application | Processes included | PSS | RSS |
|---|---|---:|---:|
| Tauri | stonks + WebKitNetworkProcess + two WebKitWebProcesses | 524.6 MiB | 853.7 MiB |
| GPUI debug | stonks-app | 187.2 MiB | 234.0 MiB |

PSS apportions shared pages between processes; summing RSS double-counts
shared mappings. These figures are a point-in-time RAM observation, **not a
feature-equivalent benchmark**. GPUI still lacks much of the composer, voice,
settings and other application behavior. GPU/device allocations are not fully
represented. The new composer was not present in this sample. Do not extrapolate
the difference into a guaranteed release memory, CPU, battery or latency gain.

An eventual comparison must use matching window dimensions, channel history,
images/animation, voice state, warm-up, foreground/background state, and release
builds. Capture idle CPU, scroll/input latency, frame times, startup, memory and
GPU allocations separately.

Architectural benefits for Stonks: native Rust views can call its Rust core
directly, share entities across windows, and consume decoded media without a
webview/JavaScript IPC boundary. GPU rendering provides control over painting;
it does not itself establish faster scrolling or lower power use. Tauri retains
the advantages of mature web controls, browser content and the existing UI.
