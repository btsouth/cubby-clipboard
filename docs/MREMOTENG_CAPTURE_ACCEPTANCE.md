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

## Fix and privacy boundaries

Capture, paste, and the Win+V helper share one executable classifier. mRemoteNG,
mstsc, and msrdc use the Windows RDP classification. Cubby retains their paired
history/cloud exclusions only when the process is the actual clipboard owner.
It leaves those flags on the Windows clipboard and does not relay that copy.
Executable lookup uses limited process-query rights, and failed metadata lookup
preserves ownership without inventing a remote classification.

`Clipboard Viewer Ignore` still excludes content, including through RDP. Local
history opt-outs, the same pair from other remote clients, isolated history
opt-outs, ignored apps, and likely-secret filtering keep their exclusion gates.
Foreground-only attribution cannot invoke either remote exception.

RDP's blanket pair cannot distinguish ordinary content from an originating
remote application's identical opt-outs. Skip sensitive cannot guarantee that
all remote passwords are excluded. The settings description explains that
remote copies can still be saved and that Ignored Apps excludes the client.

The live image test also exposed a decoder defect in the current development
build: System.Drawing's redirected V5 bitmap repeated three RGB masks after
its header. Cubby read those masks as pixels. The fix recognizes the exact
header/masks/image-size layout, preserves ordinary packed V4/V5 handling and
alpha, and includes the captured synthetic bitmap as a regression fixture.
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
| Windows restart, Cubby restart, and RDP reconnect | First 50 retained; next 50 accepted |
| Unicode and multiline remote copy | Exact stored UTF-8 payload |
| Remote RTF | Exact stored RTF and plain text |
| Remote HTML | Present at source, absent on local clipboard; redirection not validated |
| Forwarded Clipboard Viewer Ignore | Excluded; subsequent ordinary remote copy accepted |
| Local Viewer Ignore and history DWORD controls | Filtered; subsequent ordinary local copy accepted |
| RDP System.Drawing image | Initial pixel mismatch reproduced; fixed stored 2×2 image matches all four source pixels |

History checks read isolated, stopped database snapshots and compared decrypted
payload bytes against supplied synthetic fixtures. They verified retention,
not paste of every one of the 100 entries. Payloads and protected storage keys
were not written to production logs. The actual bitmap regression fails with
the old decoder and passes with the corrected decoder; existing packed V4/V5
pixel and alpha regressions also pass.

## Remaining acceptance

- Exact reporter environment and Ranger's clipboard behavior.
- Signed Microsoft Store installation/update over existing history.
- Exact paste from every retained item, including rich and image destinations.
- Local-to-remote and two-session interleaving, burst rates faster than the
  measured 1.2-second interval, interrupted redirection, and network failures.
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
