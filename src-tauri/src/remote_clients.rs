//! One remote-client classification for capture, paste, and the hotkey helper.
//! Match the clipboard owner's executable, never a title or foreground hint:
//! capture uses this classification to interpret privacy markers.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteClient {
    Generic,
    Ninja,
}

pub fn classify_remote_process(process_name: &str) -> Option<RemoteClient> {
    if process_name.eq_ignore_ascii_case("ncplayer.exe") {
        return Some(RemoteClient::Ninja);
    }

    if [
        "mstsc.exe",
        "msrdc.exe",
        // mRemoteNG hosts the Microsoft RDP control in its own process, so
        // its clipboard owner and foreground window are not mstsc.exe.
        "mremoteng.exe",
        "anydesk.exe",
        "teamviewer.exe",
        "teamviewer_desktop.exe",
        "screenconnect.clientservice.exe",
        "screenconnect.windowsclient.exe",
        "splashtop.exe",
        "strwinclt.exe",
        "rustdesk.exe",
    ]
    .iter()
    .any(|candidate| process_name.eq_ignore_ascii_case(candidate))
    {
        Some(RemoteClient::Generic)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{classify_remote_process, RemoteClient};

    #[test]
    fn recognizes_mremoteng_including_windows_filename_casing() {
        for name in ["mRemoteNG.exe", "mremoteng.exe", "MREMOTENG.EXE"] {
            assert_eq!(classify_remote_process(name), Some(RemoteClient::Generic));
        }
    }

    #[test]
    fn preserves_existing_remote_client_classifications() {
        assert_eq!(
            classify_remote_process("NCPLAYER.EXE"),
            Some(RemoteClient::Ninja)
        );
        for name in [
            "MSTSC.EXE",
            "msrdc.exe",
            "anydesk.exe",
            "teamviewer.exe",
            "teamviewer_desktop.exe",
            "screenconnect.clientservice.exe",
            "screenconnect.windowsclient.exe",
            "splashtop.exe",
            "strwinclt.exe",
            "rustdesk.exe",
        ] {
            assert_eq!(
                classify_remote_process(name),
                Some(RemoteClient::Generic),
                "{name}"
            );
        }
    }

    #[test]
    fn local_and_similarly_named_processes_are_not_remote_clients() {
        for name in [
            "notepad.exe",
            "1Password.exe",
            "",
            "mRemoteNG",
            "mRemoteNG.exe.bak",
            "fake-mRemoteNG.exe",
        ] {
            assert_eq!(classify_remote_process(name), None, "{name}");
        }
    }
}
