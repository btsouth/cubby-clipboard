# mRemoteNG history capture acceptance

## Report and reproduced cause

On September 30, 2026, a Microsoft Store user running Cubby v1.3.4 reported
missing copies from mRemoteNG. Their workflow copies a username, then a password,
pastes the password locally, and retrieves the earlier username from Cubby.
Local paste availability and the exact prior Store version were not supplied.
This describes missing new captures, rather than deletion of existing history.

A real mRemoteNG RDP session reproduced missing history with both published
portable v1.3.3 and v1.3.4. Local Windows paste returned the exact password, but
both history entries were absent. Cubby's logs confirmed sensitivity filtering.
The source clipboard had no privacy markers; the RDP client added these DWORDs:

- `CanIncludeInClipboardHistory = 0`
- `CanUploadToCloudClipboard = 0`

Cubby incorrectly treated that blanket transport pair as an exclusion from the
originating application. The remote classifiers also omitted `mRemoteNG.exe`,
selecting local paste timing and preventing the existing remote monitor-marker
exception. Source attribution requested process-memory access, which can fail
for elevated clients; failed lookup also erased explicitly observed ownership.

v1.3.4 introduced the OLE reader, sequence handling changes, and frozen-owner
recovery in #333. The reproduced text filtering defect predates that release.
These tests do not establish why the reporter first noticed it after updating.

A faster run exposed a second text-capture defect: at a requested 100 ms
between 100 remote copies, notification-driven capture retained only 15.
With Cubby stopped, a Windows Forms observer reading on sequence changes also
saw 15; an observer reading every 20 ms saw all 100. The local clipboard's text
could change without advancing its sequence. This was readable data that Cubby's
notification-only path missed, rather than evidence that RDP never delivered it.

## Fix and privacy boundaries

Capture, paste, and the Win+V helper share one executable classifier. mRemoteNG,
mstsc, and msrdc use the Windows RDP classification. Cubby retains their paired
history/cloud exclusions only when the process is the actual clipboard owner.
It leaves those flags on the Windows clipboard and does not relay that copy.
Executable lookup uses limited process-query rights, and failed metadata lookup
preserves ownership without inventing a remote classification.

After a notification identifies an actual RDP owner, the listener also samples
text every 40 ms through the existing OLE reader. Changed payloads are queued
even when their sequence stays unchanged; unchanged samples are deduplicated
before enqueue. Both notifications and samples recheck ownership and privacy metadata, and reread Unicode text after rich formats to reject a mixed copy. Sampling survives listener recreation. Local
owners use notifications, images/files are not polled, and owners that time out or exhaust their retry budget are
not sampled again until a new sequence arrives. Unreadable privacy DWORDs
are retried through the bounded OLE worker and never treated as absent flags. Startup does not import an
already-existing clipboard. Network latency, brief overwrites, and unreadable
or unforwarded content still prevent a universal capture guarantee.

`Clipboard Viewer Ignore` still excludes content, including through RDP. Local
history opt-outs, the same pair from other remote clients, isolated history
opt-outs, ignored apps, and likely-secret filtering keep their exclusion gates.
Foreground-only attribution cannot invoke either remote exception.

RDP's blanket pair cannot distinguish ordinary content from an originating
remote application's identical opt-outs. Skip sensitive cannot guarantee that
all remote passwords are excluded. The settings description explains that
remote copies can still be saved and that Ignored Apps excludes the client.

The live image test exposed an existing decoder/transport ambiguity:
System.Drawing's redirected V5 bitmap repeated three RGB masks after its
header. A candidate byte-layout heuristic restored the synthetic pixels, but
review reproduced corruption of a valid packed V5 with allocation padding.
The companion CF_DIB was already altered too, so it could not independently
establish the original pixels. That bitmap change was withdrawn. This release
keeps the standard packed decoder, preserves the padded-image regression and
both synthetic transport fixtures, and does **not** fix this image defect.
The original-pixel fixture remains failed acceptance evidence; no passing
image-fidelity test is claimed for this release.
Previously damaged captures cannot be reconstructed from the damaged pixels.

## Completed Windows run

Test date: September 30, 2026. Both endpoints used Windows 11 Enterprise,
build 26200. The local client used official portable mRemoteNG 1.76.20.24669
with Redirect Clipboard explicitly enabled. The separate, disposable Windows
RDP server used a synthetic account and fixtures; neither endpoint copied real
credentials. The cloned endpoints used RDP native authentication after their
initial NLA authentication failed. This does not validate an NLA configuration.

Published Cubby controls used their fresh portable profiles with Skip sensitive
enabled. The candidate used isolated portable storage with Skip sensitive,
Ignore ghost clips, and remote relay enabled; Skip likely secrets was disabled.
UAC was enabled for the elevation run. Token inspection confirmed elevated
mRemoteNG and unelevated Cubby. The first 50-copy run preceded that UAC change.

| Check | Result |
|---|---|
| v1.3.3 username/password control | 0/2 history entries; exact local password paste |
| v1.3.4 username/password control | 0/2 history entries; exact local password paste |
| Classifier-only candidate control | 0/2; confirmed the transport-pair fix was also needed |
| Fixed username/password workflow | Both retained; earlier username retrieved through Cubby |
| 50 unique remote text copies, 1.2 seconds apart | 50/50 exact stored payloads |
| 50 more copies, elevated mRemoteNG / unelevated Cubby | 50/50 exact stored payloads |
| 100 copies at requested 100 ms, notifications only | 15/100; same-sequence text changes reproduced |
| Cubby stopped: sequence-triggered Windows observer | 15/100; final local paste returned the 100th value |
| Cubby stopped: continuous Windows observer | 100/100 distinct values at requested 100 ms |
| Candidate with RDP text sampling, requested 100 ms | 100/100 exact stored payloads |
| Earlier sampling candidate: repeat requested 100 ms run | 100/100; all 300 text fixtures retained across the completed batches |
| Windows restart, Cubby restart, and RDP reconnect | First 50 retained; next 50 accepted |
| Unicode and multiline remote copy | Exact stored UTF-8 payload |
| Remote RTF | Exact stored RTF and plain text |
| Remote HTML | Present at source, absent on local clipboard; redirection not validated |
| Forwarded Clipboard Viewer Ignore | Excluded; subsequent ordinary remote copy accepted |
| Local Viewer Ignore and history DWORD controls | Filtered; subsequent ordinary local copy accepted |
| mRemoteNG in Ignored Apps | Remote fixture excluded; removing the setting restored capture |
| RDP System.Drawing image | Existing mismatch reproduced; heuristic candidate restored pixels, but withdrawn after review found valid padded-image corruption |
| Earlier username after the password, RDP client closed | Exact paste into an empty local Windows rich text box |
| Retained RTF, RDP client closed | Exact plain text and bold formatting pasted locally |
| Retained image from the withdrawn bitmap candidate, RDP client closed | Restored all four exact stored source pixels; final release bitmap capture is not validated |

History checks read isolated, stopped database snapshots and compared decrypted
payload bytes against supplied synthetic fixtures. They verified retention,
not paste of every one of the 300 entries. Payloads and protected storage keys
were not written to production logs. The image-fix candidate passed its source-pixel fixture but failed the independent
padded-image review fixture, so it was withdrawn. Standard packed V4/V5 pixel,
alpha, and allocation-padding regressions pass.

## Adversarial review corrections

Independent review found and corrected restart sampling loss, queued-source
exit/PID lookup loss, mixed same-sequence rich reads, notification privacy
recheck gaps, and exhausted polling retries. The padded-bitmap review finding caused the
unverified image change to be withdrawn rather than shipped.
Source executable/path are frozen during capture; icon/version metadata stay
on the consumer. Privacy DWORD reads now share the bounded OLE worker and a
failed read defers or records a failed capture instead of permitting relay.
The reader timeout tests remain serialized around their shared worker.

The coherence rechecks reduce race windows; they cannot reconstruct content
that was overwritten before a read or prove atomic generations when the RDP
transport supplies neither a sequence change nor an immutable data object.

## Fresh reviewed-source validation

The reviewed source passed Windows CI, including tests, Clippy, release checks,
and the x64/ARM64 default and app-store compile matrix. Fresh isolated Windows
runs on source `0fc4cc1` retained 100/100 unique remote copies at the requested
100 ms interval, one first remote copy after restarting Cubby, 50/50 exact text
and RTF payload pairs, and a 1.08 MB text/RTF payload. Source `3a1b9dc` then
retained all six same-text copies with distinct RTF formatting and the same
large text/RTF payload. Its only functional difference is streaming the existing
fingerprint digest without building a combined payload buffer; compatibility
with the original digest is independently tested.

Sampling continues while an RDP owner holds text. In this four-vCPU VM, an
unchanged small text/RTF copy used about 4.7% of one CPU core over eight seconds.
An unchanged 1.08 MB text/RTF copy used 20.1% on `0fc4cc1` and 19.1% on `3a1b9dc`,
with approximately 45 MB working set in the latter run. These short samples do
not establish a significant performance improvement. Idle backoff or skipping
rich formats was not applied because those changes can lose copies after idle
or changes that affect only formatting. Persistent polling cost remains a
performance limitation.

## Signed installer upgrade smoke

The signed private candidate from `0fc4cc1` upgraded the existing x64 v1.3.5
installation to v1.3.6. The slim installer and offline Store-channel installer
both installed successfully. Exactly `cubby.exe` and `uninstall.exe` were
installed, with valid Authenticode signatures. This is NSIS channel-package
validation, not an update performed by the Microsoft Store client.

All 36 seeded pre-upgrade history records retained their UUID, content hash,
encrypted content, and active status, and the protected storage key was
unchanged. The packaged build captured a fresh remote username/password pair;
the password pasted locally and the earlier username pasted exactly from Cubby
into an empty local Windows rich text box. A forwarded Clipboard Viewer Ignore
fixture was excluded and the following ordinary copy was retained. Compact
view persisted across cold restart, Win+V opened the installed flyout, and
autostart added and removed its installed-path Run entry correctly.

## Remaining acceptance

- Exact reporter environment and Ranger's clipboard behavior.
- Nonstandard redirected V5 image layout and original-pixel provenance; existing defect remains.
- Signed Microsoft Store installation/update over existing history.
- Exact paste from every retained item, including rich and image destinations.
- Local-to-remote and two-session interleaving, burst rates faster than the
  requested 100 ms interval, interrupted redirection, and network failures.
- Lock/unlock, sleep/resume, large images, and other mRemoteNG versions.
- Per-client validation of every other recognized remote product.

The completed synthetic runs establish the reproduced defects and their fixes;
they do not establish universal reliability or the Store update path.

## References

- [mRemoteNG RDP configuration](https://mremoteng.readthedocs.io/en/v1.77.3-dev/protocols/rdp.html)
  documents Redirect Clipboard.
- [Microsoft clipboard formats](https://learn.microsoft.com/en-us/windows/win32/dataxchg/clipboard-formats)
  defines Windows history/cloud exclusion formats.
- [Microsoft process-image lookup](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-queryfullprocessimagenamew)
  documents limited process-query rights for executable paths.
