# Native Music card

The visual reference is `src/features/voice/components/MusicCard.tsx`, as mounted
by `VoiceDock.tsx`. The native dock owns the card placement and layout; the
shell mounts the playlist at the window overlay root.

- `session.rs` bridges the per-call `VoiceSession` music watch and async
  requests/controls to one observed card per connected call. It resolves
  requester names from the connected channel's server and loads artwork through
  the shared image cache. Playback position comes from core, not a UI clock.
- The worker serializes operations and supplies a unique seek nonce with the
  observed item, revision, and playback epoch. Local volume uses the shared
  voice listening gain and persists in `voice-listening.json` (default 70%).
- The composer routes recognized, attachment-free `@Music` commands to the
  active call instead of chat. It clears an accepted draft only if the call,
  composer navigation, and draft revision still match. Failure or stale
  completion keeps the draft; playlist Add failure likewise keeps the URL.

The native title truncates rather than animating. The shared native dialog
scrolls the panel on short windows rather than pinning the footer and scrolling
only the queue. Native range, input, focus and dock geometry still need
side-by-side visual verification; live-room playback was not verified here.
