# Settings host integration

The host owns headings, scrolling, navigation, and authenticated teardown. Mount the page against an `AccountSettingsService` pinned to the same authenticated context as the gateway. The current session ID comes from the matching stored credential, never from the device list. An unidentified session cannot be revoked individually; bulk revoke sends `revoke_self=false`.

Busy pages block navigation, close, and logout; dirty pages use the host's discard dialog. `ProfileSaved` and `UsernameChanged` trigger a canonical current-user refresh; `UserUnblocked` updates the session user projection.

- `set_account_actions(open_sessions, account_ended)` enables the Sessions link and the two management actions. Route the first callback through the normal Settings navigation. The second runs only after a successful disable/delete and must clear the matching authenticated session locally, even if remote logout would fail. Capture the matching session identity and use weak host handles. Without these hooks, the controls remain disabled.
- `set_privacy_preferences(snapshot, writer)` wires typing, embeds, external-link warnings, and clearing native trusted link origins. The native opener enforces warnings for all external link activations. Linux desktop notification delivery reads the shared notification content preference before building previews.
- Profile image selections are staged until Save. Avatar uses the existing square cropper; background uses native image selection and a cover preview, but does not yet have Tauri's interactive 16:9 crop. MFA enrollment and recovery-code management are outside this change.
