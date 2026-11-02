# Shortcut behavior

## Defaults

- New installations use `Win+Alt+V`.
- Win+V replacement is enabled by default and can be disabled in Settings.
- The shortcut is customizable.
- If the configured shortcut is unavailable at startup, Cubby uses `Win+Ctrl+Alt+V` for now and keeps trying to get the configured one back for ten minutes. The saved setting does not change, and the next launch tries it again.
- A shortcut conflict at startup does not turn off Win+V replacement, because the helper does not need a global shortcut to reach Cubby.
- Changing to a conflicting shortcut keeps the previous working shortcut and returns a visible error.

## Win+V

Windows reserves `Win+V` for Clipboard History and rejects attempts to register
it through the normal global-hotkey API. Cubby therefore handles replacement in
an isolated helper mode instead of installing a keyboard hook on the main UI
process.

The helper is a second instance of Cubby's own executable launched before the
normal application startup path. No separate runtime or remapping application is
required. The helper intercepts only an exact physical `Win+V` chord and signals
the main Cubby process through a loopback-only activation channel. The channel
accepts only a per-session token that Cubby hands the helper it spawned, from a
loopback source, and only at a human-scale rate. A bare `activate` datagram is
ignored. The token travels on the helper command line, so a same-user process
that can read Cubby's arguments can still forge an activation; that is weaker
than an attacker who can decrypt `storage.key`, but it is no longer an
unauthenticated display path.

The helper ignores its own injected events, restores the physical Windows-key
state before unrelated input continues, and exits independently if Cubby
disables replacement mode.

Windows calls the newest low-level keyboard hook first, and a hook that does not
call the next one hides the key from older hooks while Windows still acts on it.
Apps that start after Cubby at sign-in can do this, so for five minutes after it
starts the helper moves its hook back to the front every two seconds. It skips
this while a supported remote client is in the foreground, and stops after five
minutes so a remote client opened later keeps priority (see below).

Cubby monitors the helper and restarts it after an unexpected exit. The helper
also monitors its parent process, so a Cubby crash cannot leave the hook behind.
Disabling replacement or exiting Cubby restores native Windows Clipboard History.
Cubby remains fully functional with the configured global shortcut when
replacement mode is disabled.

## Remote-session trigger

A global shortcut registered locally does not fire while a remote-control client
is focused, because the client captures keyboard input first. The same low-level
hook that intercepts `Win+V` also matches the user's configured Cubby hotkey when
a known remote client is the foreground application, and suppresses the chord so
it does not reach the remote session, whatever the user set it to (for example
Ctrl+backtick).

This works only when the remote client's own keyboard forwarding is off. Clients
such as Ninja Remote install their own keyboard capture when forwarding is on,
and because that capture loads after Cubby's it runs ahead of Cubby's hook and
consumes the keys first. With forwarding off, Cubby's hook wins. When forwarding
must stay on, open Cubby from the tray instead. In the helper's first five
minutes, a client that loses the foreground can end up behind Cubby's hook until
it next installs its capture.

The trigger is active only for supported remote clients, including Ninja Remote,
Windows Remote Desktop, AnyDesk, TeamViewer, ScreenConnect, Splashtop, and
RustDesk, and only while replacement mode is enabled (that is what runs the
hook). It does nothing in ordinary local applications, where the normal global
shortcut already handles activation.
