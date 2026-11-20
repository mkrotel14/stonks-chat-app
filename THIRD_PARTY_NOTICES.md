# Third-party notices

The root `LICENSE` covers Stonks Chat project work, including the newly generated branding and cues. It does not replace upstream licenses or grant trademark rights.

| Vendored crate | Declared license and source |
| --- | --- |
| `gpui-pre-wgpu` 0.3.12 | Apache-2.0; retained [`LICENSE-APACHE`](vendor/gpui-pre-wgpu/LICENSE-APACHE) from the Zed snapshot described in [`README.stonks.md`](vendor/gpui-pre-wgpu/README.stonks.md). |
| `livekit` 0.9.2 | Apache-2.0; [`LICENSE`](vendor/livekit/LICENSE) from LiveKit Rust SDK revision `5a656c4bc4bc5c4f7fad17ab69044db3b487dfeb`. |
| `webrtc-sys` 0.3.46 | Apache-2.0; [`LICENSE`](vendor/webrtc-sys/LICENSE) from the same SDK revision. [`NOTICE.md`](vendor/webrtc-sys/NOTICE.md) and embedded notices cover separately licensed code and build inputs; the prebuilt WebRTC library has its own upstream terms. |
| `livekit-protocol` 0.7.13 | Apache-2.0; [`LICENSE`](vendor/livekit-protocol/LICENSE) from SDK revision `8066415f8faa09a0ec0a6643e6cfdaa825167c65`. The exact crate archive also ships [`protocol/LICENSE`](vendor/livekit-protocol/protocol/LICENSE). |

The LiveKit revisions come from `.cargo_vcs_info.json` in the matching crates.io archives. Other Cargo dependencies retain their own terms.

- Inter fonts: [SIL OFL 1.1](crates/stonks-ui/assets/fonts/OFL-Inter.txt). The generated inline-object font retains its [existing terms](crates/stonks-ui/assets/fonts/LICENSE-Stonks-Inline.txt).
- Lucide icons: see the [retained upstream license](crates/stonks-ui/assets/icons/LICENSE).
- Twitter/Twemoji graphics: [CC BY 4.0](crates/stonks-ui/assets/emoji/LICENSE-GRAPHICS); emoji-data metadata/code: [MIT](crates/stonks-ui/assets/emoji/LICENSE-emoji-data.txt). See [artwork attribution](crates/stonks-ui/assets/emoji/NOTICE.md).
- Media packages may carry separate executables: [BtbN FFmpeg](https://github.com/BtbN/FFmpeg-Builds) LGPL build (including ffprobe), [yt-dlp](https://github.com/yt-dlp/yt-dlp) under The Unlicense, and [Deno](https://github.com/denoland/deno) under MIT. Standalone binaries can include separately licensed components. See each upstream release for notices, configuration, corresponding source and build instructions. The Linux recipes do not bundle these tools. Review the exact binaries and redistribution obligations before distributing any package.
