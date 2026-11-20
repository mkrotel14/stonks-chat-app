# GPUI compact home navigation

The authenticated GPUI shell uses the Android Compose home flow below 800 logical pixels. The desktop rail, sidebars, resize handles and settings frame remain the wide layout. Resize preserves each layout's navigation selection in memory; changing the authenticated identity resets compact navigation.

## Source map

Android paths below are relative to `stonks-kmp/androidApp/src/main/kotlin/com/rdxlabs/stonks/android/`. GPUI paths are relative to `crates/stonks-app/src/features/`.

| Android reference | GPUI port |
| --- | --- |
| `navigation/HomeNavigation.kt`, `home/ui/HomeShellScreen.kt` | `shell/compact.rs` and `shell/render.rs`: Messages starts at its root; switching tabs restores their details, reselecting returns to the selected tab's root, and Back from another root returns to Messages. Main detail screens retain the tab bar. |
| `home/ui/ConversationsScreen.kt`, `home/ui/ChatDetailScreen.kt` | Messages lists accessible private conversations. Selecting a row opens the existing conversation pane, including real timeline, composer, search and pins. |
| `home/ui/ServersScreen.kt`, `home/ui/ChannelListScreen.kt`, `home/ui/ChannelChatScreen.kt` | Servers → server channels → text chat. Reuses live category, unread, permission, member, and channel-action logic. Joining voice does not push a chat route. Removed or inaccessible detail targets show an unavailable surface with Back. |
| `home/ui/settings/SettingsScreens.kt`, `home/ui/ProfileScreen.kt` | You shows the native settings index; supported entries open detail pages with no tab bar. Back returns to the index through the existing busy/unsaved-change guard. Logout remains the real session action. |
| `home/ui/VoiceDock.kt`, the bottom overlay in `HomeShellScreen.kt` | The existing GPUI dock floats above the centered navigation pill. Lists and chat scroll beneath the bottom fade, with an inset for the floating controls. The composer is painted in front of the fade and remains above the dock. It uses the real connection phase and mute, deafen, disconnect, camera, screen share, and soundboard controls. |

## Deliberate differences and remaining gaps

- This is navigation and layout parity, not a pixel-for-pixel Material 3 port. It uses GPUI theme tokens, shared buttons, desktop window chrome, and existing feature bodies. Some new compact labels are English-only. Android process-death state restoration is not implemented.
- Android conversation filters/friends and create/join server affordances were not added to the compact shell. No placeholder actions were introduced. The Android channel switcher and long-press action sheet are not cloned; GPUI uses Back to the channel list and its existing context menu.
- The You index reuses supported native desktop preferences, including desktop-only pages. Android app permissions is not replicated. Server/channel settings still use their existing native compact frames and page menus instead of separate Compose menu/detail route stacks. Those frames cover the voice card while open. The You index uses the measured overlay inset as bottom padding inside its scroll content, so it scrolls behind the pill like the other tabs.
- The voice card reuses GPUI controls and channel/status display, not Android's participant avatar strip, now-playing overlay, or soundboard settings route from the dock. Android system insets and predictive Back are not applicable to this desktop port.
- Unit tests cover independent tab history, reselection, replacement of an active text detail, and Back destinations. The selected app suite does not exercise rendered layout or real voice; visual checks at matching viewport sizes and a live connected-voice smoke test remain outstanding.
