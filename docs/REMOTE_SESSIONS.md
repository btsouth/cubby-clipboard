# Remote-session behavior

Remote-control applications handle keyboard and clipboard input differently from
ordinary Windows applications. Cubby detects supported remote clients when the
flyout opens and selects a compatible paste strategy.

## Opening Cubby inside a remote session

Cubby matches your configured hotkey inside the same low-level keyboard hook that
powers its `Win+V` replacement, so the hotkey that opens the flyout locally also
opens it while a supported remote client is focused, and the chord is suppressed
so it does not leak into the remote machine. The trigger tracks whatever hotkey
you set, so a user coming from Ditto can keep Ctrl+backtick.

Two requirements, both verified by testing:

- **Win+V replacement must be enabled** in Settings. That is what runs the hook.
- **The remote client's own keyboard forwarding must be off.** When Ninja Remote's
  "forward keyboard shortcuts" is on, Ninja installs its own keyboard capture that
  runs ahead of Cubby's hook and consumes the keys first, so the hotkey never
  reaches Cubby. With forwarding off, Cubby's hook wins and the hotkey works. Avoid
  `Win+V` itself inside a remote session with forwarding on: the Windows shell
  handles the Win key low enough that it can both open Cubby locally and leak to
  the remote.

If you keep keyboard forwarding on, open Cubby from the tray instead of a hotkey.

## Ninja Remote

Recommended player settings:

- Enable clipboard synchronization.
- Turn **Forward keyboard shortcuts off** if you want to open Cubby with a hotkey
  inside the session. With it on, use the tray icon instead.

Recommended Cubby setting:

- Remote session paste: **Copy, then Ctrl+V**.

Ninja filters injected keyboard input, so Cubby cannot press paste for you inside
a Ninja session. The reliable, driver-free flow is to let Ninja synchronize the
clipboard as data and paste it with a physical `Ctrl+V`:

Workflow:

1. Focus the destination inside Ninja Remote.
2. Open Cubby (hotkey with forwarding off, otherwise the tray icon).
3. Select a clip in Cubby.
4. Cubby writes the complete item to the local clipboard, closes, restores focus
   to Ninja.
5. Press physical `Ctrl+V`.

This is the preferred mode for large logs, scripts, formatted text, and other
large clipboard items. Ninja synchronizes the clipboard as data, so content is
pasted at once, not typed character by character.

The optional **Paste as keystrokes** mode invokes Ninja's own toolbar action
through Windows accessibility. It is useful for short text or sensitive values,
but Ninja deliberately types the content into the remote session and is
therefore unsuitable for large items.

Cubby does not synthesize Ctrl+V for Ninja. Ninja's player accepts physical
keyboard input for forwarding but ignores the synthetic input methods available
to ordinary Windows applications.

## Other remote clients

Cubby recognizes Windows Remote Desktop, mRemoteNG, AnyDesk, TeamViewer, ScreenConnect,
Splashtop, and RustDesk. These clients currently use focus restoration followed
by Cubby's standard Ctrl+V compatibility path. Each client should be validated
individually because forwarding and clipboard policies vary.

Capture, paste strategy, and the hotkey helper share the same executable
classification. mRemoteNG hosts RDP in `mRemoteNG.exe`; treating only
`mstsc.exe` as RDP misses that owner. A recognized remote owner's blanket
`ExcludeClipboardContentFromMonitorProcessing` marker does not suppress history,
while `Clipboard Viewer Ignore` and the ignored-app setting still apply.

The Windows RDP control also adds `CanIncludeInClipboardHistory = 0` and
`CanUploadToCloudClipboard = 0` to ordinary remote copies. Cubby retains those
copies when the actual owner is mRemoteNG, mstsc, or msrdc, and leaves the
Windows clipboard flags untouched. The same pair from a local application or a
different remote client remains excluded with Skip sensitive enabled; an
isolated history opt-out also remains excluded. Foreground attribution never
qualifies for an exception.

Some RDP text changes are readable before another clipboard notification or
sequence advance. After Windows announces a copy owned by a recognized RDP
client, Cubby also samples its text every 40 ms, keeping changed payloads and
deduplicating unchanged samples. Each sample rechecks ownership and privacy
markers; local owners return to notification-driven capture. Sampling excludes
images and files and stops on a timed-out owner until a new sequence arrives.
It does not import the clipboard that existed before Cubby started.

RDP's blanket pair cannot distinguish an ordinary remote copy from a source
application's identical opt-outs. Skip sensitive therefore cannot promise to
exclude every remote password. A forwarded `Clipboard Viewer Ignore` marker
still excludes it. Use Ignored Apps to exclude the remote client entirely, or
Skip likely secrets for Cubby's separate content-based filtering.

For mRemoteNG, enable **Redirect Clipboard** on the connection. If a remote
copy cannot paste into a local application, clipboard redirection has not made
the content available to Cubby. See the [mRemoteNG acceptance run](MREMOTENG_CAPTURE_ACCEPTANCE.md)
for the checks needed to confirm this integration on Windows.

## Testing

- `cargo test --all-targets` covers remote-client detection, trigger state, and
  paste-mode selection.
- `scripts/test-remote-trigger.ps1` validates the helper's direct activation
  channel and the configured-hotkey trigger (Ctrl+backtick) without requiring
  Ninja.
- `scripts/test-paste-compat.ps1` exercises standard and generic remote focus and
  paste behavior in an isolated Windows test window.
