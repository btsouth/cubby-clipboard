# mRemoteNG history capture acceptance

## Report and confirmed code defects

On September 30, 2026, a user running the Microsoft Store build of Cubby v1.3.4
reported that copies from mRemoteNG never appeared in history, including after
reconnecting the session and restarting Cubby. Whether those copies could paste
into a local application was not supplied. This report describes missing new
captures; it does not establish that existing database history was deleted.

The v1.3.4 remote-client classifiers omitted `mRemoteNG.exe`. That selected local
paste timing and, with Skip sensitive enabled, discarded copies carrying only
a remote viewer's blanket monitor-exclusion marker. The clipboard owner was
also identified using process-memory read rights, which can fail for elevated
applications; a failed lookup incorrectly erased explicit ownership and could
trigger Ignore ghost clips.

The fix shares one remote-client classifier across capture, paste, and the
hotkey helper, adds mRemoteNG, and reads executable paths with
`PROCESS_QUERY_LIMITED_INFORMATION`. Failed metadata resolution preserves the
ownership observed by `GetClipboardOwner`. It never grants a remote privacy
bypass based on the foreground window.

## Automated coverage

- Exact executable matching, Windows filename casing, all previously supported
  viewers, and rejection of local or similarly named processes.
- mRemoteNG captures with a viewer's blanket exclusion marker are eligible for
  history. `Clipboard Viewer Ignore` and `CanIncludeInClipboardHistory = 0`
  remain excluded under Skip sensitive, including alongside the blanket marker.
- Foreground-only attribution remains excluded and cannot trigger a relay.
  Relay disabled and ignored-app cases remain excluded from relay.
- mRemoteNG selects the existing 600 ms remote paste synchronization delay.
- Windows source-path lookup and explicit-ownership preservation on lookup failure.
- The hotkey helper recognizes mRemoteNG through the shared classifier.

These are policy and source-attribution regressions, not a completed live RDP
or Store-installation test.

## Live Windows acceptance (pending)

Use a Windows test account and non-sensitive synthetic content. Record Cubby
version and distribution, Windows versions on both ends, mRemoteNG version,
clipboard-redirection settings, and whether mRemoteNG is elevated. Keep Skip
sensitive and Ignore ghost clips enabled. Enable Redirect Clipboard on the
connection; ensure mRemoteNG is not in Cubby's ignored applications.

1. Copy a unique numbered marker in the remote session and paste into local
   Notepad. Verify the exact text, then verify Cubby retains it. Capture failure
   after a successful local paste implicates Cubby; failure of local paste also
   requires investigating redirection. Collect Cubby's local capture diagnostics
   and reason for any filtered capture without collecting real clipboard data.
2. Copy 50 distinct numbered text markers from remote to local at normal working
   speed. Require 50/50 history entries and exact paste from each retained item.
   Repeat local to remote. Repeat with mRemoteNG elevated and Cubby unelevated.
3. Copy Unicode, multiline text, HTML/RTF from a rich editor, and screenshots.
   Confirm retained text and image pixels, and rich paste into a compatible
   editor. Re-copy identical content and confirm the existing row is refreshed.
4. Copy before disconnect, close the source application/session, and paste the
   retained item from Cubby. Reconnect, copy new markers immediately, restart
   Cubby, and verify both retained and newly copied items. Repeat after Windows
   lock/unlock and sleep/resume. Where possible, include two simultaneous RDP
   sessions and alternating local/remote copies.
5. Install the fixed signed candidate through the intended Store update path
   over a test v1.3.4 installation. Verify existing history survives the update
   and restart, then repeat the first two checks. A standalone development build
   does not verify the Store update path.
6. Verify deliberate exclusions with synthetic privacy-tagged fixtures and an
   ignored mRemoteNG process. Disable redirection and confirm the integration
   does not claim it captured remote content that never reached the local
   clipboard; re-enable it and confirm subsequent captures work.

Record counts, paste outcomes, and diagnostics for any miss. A successful
synthetic test or generic remote paste test must not be reported as a completed
mRemoteNG session run. This fix addresses verified code defects; confirmation
of the original reporter's cause and live Windows acceptance remain pending.

## References

- [mRemoteNG RDP configuration](https://mremoteng.readthedocs.io/en/v1.77.3-dev/protocols/rdp.html)
  documents Redirect Clipboard.
- [Microsoft clipboard formats](https://learn.microsoft.com/en-us/windows/win32/dataxchg/clipboard-formats)
  defines the history-exclusion formats.
- [Microsoft process-image lookup](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-queryfullprocessimagenamew)
  documents limited process-query rights for executable paths.
