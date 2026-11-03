use once_cell::sync::Lazy;
use std::str::FromStr;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

pub const WIN_V_BRIDGE_SHORTCUT: &str = "Win+Alt+V";
const STARTUP_FALLBACK_HOTKEY: &str = "Win+Ctrl+Alt+V";
const STARTUP_RECOVERY_WINDOW: Duration = Duration::from_secs(10 * 60);
const STARTUP_RECOVERY_MAX_DELAY: Duration = Duration::from_secs(30);

#[derive(Clone, Default, PartialEq, Eq)]
struct ShortcutConfiguration {
    hotkey: Option<String>,
    replace_win_v: bool,
}

static CURRENT_CONFIGURATION: Lazy<Mutex<ShortcutConfiguration>> =
    Lazy::new(|| Mutex::new(ShortcutConfiguration::default()));

/// What is registered with the plugin right now, so it can be released without
/// its `unregister_all`. That call holds the plugin's lock while it waits for the
/// main thread, and the main thread takes the same lock to dispatch a hotkey
/// press, so a press at the wrong moment would hang Cubby for good.
static REGISTERED_SHORTCUTS: Lazy<Mutex<Vec<Shortcut>>> = Lazy::new(|| Mutex::new(Vec::new()));

/// What startup shortcut recovery did, surfaced to the flyout as a toast.
/// Recovery is session-only — the user's configured hotkey is never persisted
/// over — so the user must be told which hotkey is actually active.
#[derive(Clone, serde::Serialize)]
pub struct HotkeyStartupNotice {
    /// "fallback_hotkey" | "win_v_disabled" | "failed"
    pub kind: String,
    pub requested_hotkey: String,
    pub active_hotkey: Option<String>,
}

static STARTUP_NOTICE: Lazy<Mutex<Option<HotkeyStartupNotice>>> = Lazy::new(|| Mutex::new(None));

/// Keeps startup recovery and a settings save from changing shortcuts at once,
/// so recovery can never reapply settings the user has just replaced.
static SHORTCUT_CHANGES: Mutex<()> = Mutex::new(());

pub fn lock_shortcut_changes() -> MutexGuard<'static, ()> {
    SHORTCUT_CHANGES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn record_startup_notice(kind: &str, requested_hotkey: &str, active_hotkey: Option<&str>) {
    *STARTUP_NOTICE.lock().unwrap() = Some(HotkeyStartupNotice {
        kind: kind.to_string(),
        requested_hotkey: requested_hotkey.to_string(),
        active_hotkey: active_hotkey.map(str::to_string),
    });
}

/// One-shot: the flyout asks for this whenever it gains focus, so a toast never
/// expires in a hidden window and a problem that recovered is never reported.
#[tauri::command]
pub fn get_hotkey_startup_notice() -> Option<HotkeyStartupNotice> {
    STARTUP_NOTICE.lock().unwrap().take()
}

/// The shortcut registered this session, which can differ from the saved
/// settings after startup recovery (fallback hotkey, Win+V disabled).
#[derive(Clone, serde::Serialize)]
pub struct ActiveShortcut {
    pub hotkey: Option<String>,
    pub replace_win_v: bool,
}

#[tauri::command]
pub fn get_active_shortcut(app: AppHandle) -> ActiveShortcut {
    let current = CURRENT_CONFIGURATION.lock().unwrap().clone();
    ActiveShortcut {
        hotkey: current.hotkey,
        replace_win_v: app
            .state::<Arc<crate::win_v_replacement::WinVReplacementManager>>()
            .is_enabled(),
    }
}

fn parser_hotkey(hotkey: &str) -> String {
    hotkey
        .split('+')
        .map(|token| {
            let trimmed = token.trim();
            if trimmed.eq_ignore_ascii_case("win") || trimmed.eq_ignore_ascii_case("windows") {
                "Super"
            } else {
                trimmed
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

pub fn toggle_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let visible_and_focused =
            window.is_visible().unwrap_or(false) && window.is_focused().unwrap_or(false);
        crate::request_flyout_toggle(&window, visible_and_focused);
    }
}

pub fn register_shortcuts(
    app: &AppHandle,
    hotkey: &str,
    replace_win_v: bool,
) -> Result<(), String> {
    let parser_value = parser_hotkey(hotkey);
    let shortcut = Shortcut::from_str(&parser_value)
        .map_err(|error| format!("Invalid shortcut: {error:?}"))?;
    let bridge = if replace_win_v && !hotkey.eq_ignore_ascii_case(WIN_V_BRIDGE_SHORTCUT) {
        Some(
            Shortcut::from_str(&parser_hotkey(WIN_V_BRIDGE_SHORTCUT))
                .map_err(|error| format!("Invalid Win+V bridge shortcut: {error:?}"))?,
        )
    } else {
        None
    };
    let requested = ShortcutConfiguration {
        hotkey: Some(hotkey.to_string()),
        replace_win_v,
    };
    let previous = CURRENT_CONFIGURATION.lock().unwrap().clone();
    if previous == requested {
        return Ok(());
    }

    release_registered(app)
        .map_err(|error| format!("Could not release the previous shortcut: {error}"))?;

    let registration = register_one(app, shortcut).map_err(|error| format!("{hotkey}: {error}"));

    if let Err(error) = registration {
        let _ = release_registered(app);
        if let Err(restore_error) = register_configuration_without_fallback(app, &previous) {
            log::error!(
                "SHORTCUT: Failed to restore previous configuration after conflict: {}",
                restore_error
            );
            // Nothing is registered now. Forget the cached configuration so a
            // later request for it registers again instead of returning early,
            // and bring back the tray icon as the way into Cubby.
            *CURRENT_CONFIGURATION.lock().unwrap() = ShortcutConfiguration::default();
            crate::set_shortcut_ready(false);
            crate::apply_tray_visibility(app);
        } else {
            *CURRENT_CONFIGURATION.lock().unwrap() = previous;
        }
        return Err(format!(
            "{hotkey} is unavailable. Another application or Windows may already be using it. ({error})"
        ));
    }
    if let Some(bridge) = bridge {
        register_bridge(app, bridge);
    }

    *CURRENT_CONFIGURATION.lock().unwrap() = requested;
    crate::set_shortcut_ready(true);
    if replace_win_v {
        log::info!(
            "SHORTCUT: Registered {} with Win+V replacement bridge {}",
            hotkey,
            WIN_V_BRIDGE_SHORTCUT
        );
    } else {
        log::info!("SHORTCUT: Registered {}", hotkey);
    }
    Ok(())
}

/// What registered at launch. `complete` is false when the saved hotkey is not
/// the one registered, which is what startup recovery retries.
pub struct StartupRegistration {
    pub active_hotkey: Option<String>,
    pub complete: bool,
}

/// Registers the saved shortcuts, settling for less when something is taken.
/// Nothing here touches the saved settings, and none of it decides whether the
/// Win+V helper runs: the helper signals Cubby over the loopback channel, so a
/// hotkey conflict must not take Win+V away from the user.
pub fn register_at_startup(
    app: &AppHandle,
    hotkey: &str,
    replace_win_v: bool,
) -> StartupRegistration {
    let error = match register_shortcuts(app, hotkey, replace_win_v) {
        Ok(()) => {
            return StartupRegistration {
                active_hotkey: Some(hotkey.to_string()),
                complete: true,
            }
        }
        Err(error) => error,
    };
    log::error!("SHORTCUT: Startup registration failed: {}", error);

    let active_hotkey = if hotkey != STARTUP_FALLBACK_HOTKEY
        && register_shortcuts(app, STARTUP_FALLBACK_HOTKEY, false).is_ok()
    {
        log::warn!(
            "SHORTCUT: Fell back to {} for this session",
            STARTUP_FALLBACK_HOTKEY
        );
        Some(STARTUP_FALLBACK_HOTKEY.to_string())
    } else {
        None
    };
    StartupRegistration {
        active_hotkey,
        complete: false,
    }
}

/// The toast a launch owes the user, if any.
pub fn startup_notice_kind(
    requested_hotkey: &str,
    active_hotkey: Option<&str>,
    helper_failed: bool,
) -> Option<&'static str> {
    match active_hotkey {
        None => Some("failed"),
        Some(active) if active != requested_hotkey => Some("fallback_hotkey"),
        Some(_) if helper_failed => Some("win_v_disabled"),
        Some(_) => None,
    }
}

/// How long to wait before the next recovery attempt, or `None` to give up.
/// Whatever holds a shortcut at sign-in usually lets go within seconds; a
/// lasting conflict is left for the next launch or a settings change.
fn startup_recovery_delay(attempt: u32, elapsed: Duration) -> Option<Duration> {
    (elapsed < STARTUP_RECOVERY_WINDOW).then(|| {
        Duration::from_secs(2_u64.saturating_pow(attempt + 1)).min(STARTUP_RECOVERY_MAX_DELAY)
    })
}

/// Keeps trying to reach the saved shortcut settings after a launch that fell
/// short, because a conflict at sign-in would otherwise last the whole session
/// with the setting still showing as on (#376). The hotkey and the helper are
/// retried separately so a lasting hotkey conflict cannot cost the user Win+V.
pub fn spawn_startup_recovery(app: AppHandle, hotkey: String) {
    std::thread::spawn(move || {
        let started = Instant::now();
        let mut attempt = 0;
        while let Some(delay) = startup_recovery_delay(attempt, started.elapsed()) {
            std::thread::sleep(delay);
            attempt += 1;

            let _changes = lock_shortcut_changes();
            let settings = app
                .state::<Arc<crate::settings_manager::SettingsManager>>()
                .get();
            if settings.hotkey != hotkey {
                // The user chose another hotkey, so the notice is out of date.
                STARTUP_NOTICE.lock().unwrap().take();
                log::info!("SHORTCUT: Startup recovery stopped; the hotkey changed");
                return;
            }

            let replace_win_v = settings.replace_win_v;
            let registered = register_shortcuts(&app, &hotkey, replace_win_v);
            let helper_hotkey = match &registered {
                Ok(()) => hotkey.clone(),
                Err(_) => CURRENT_CONFIGURATION
                    .lock()
                    .unwrap()
                    .hotkey
                    .clone()
                    .unwrap_or_else(|| hotkey.clone()),
            };
            let replacement = app.state::<Arc<crate::win_v_replacement::WinVReplacementManager>>();
            let helper = if replacement.is_available() {
                replacement.configure(replace_win_v, Some(helper_hotkey))
            } else {
                Ok(())
            };
            if let Err(error) = registered.as_ref().and(helper.as_ref()) {
                log::debug!("SHORTCUT: Startup recovery attempt failed: {}", error);
                continue;
            }

            if replace_win_v && !replacement.is_available() {
                record_startup_notice("win_v_disabled", &hotkey, Some(&hotkey));
            } else {
                STARTUP_NOTICE.lock().unwrap().take();
            }
            crate::apply_tray_visibility(&app);
            log::info!(
                "SHORTCUT: Recovered the saved shortcut settings after {:?}",
                started.elapsed()
            );
            return;
        }
        log::warn!("SHORTCUT: Startup recovery gave up; the conflict is still there");
    });
}

fn release_registered(app: &AppHandle) -> Result<(), String> {
    let registered = std::mem::take(&mut *REGISTERED_SHORTCUTS.lock().unwrap());
    if registered.is_empty() {
        return Ok(());
    }
    app.global_shortcut()
        .unregister_multiple(registered)
        .map_err(|error| format!("{error:?}"))
}

/// The helper reaches Cubby over the loopback channel, so the bridge is only a
/// convenience and another app holding it must not fail anything.
fn register_bridge(app: &AppHandle, bridge: Shortcut) {
    if let Err(error) = register_one(app, bridge) {
        log::warn!(
            "SHORTCUT: {} is unavailable; Win+V replacement does not need it: {}",
            WIN_V_BRIDGE_SHORTCUT,
            error
        );
    }
}

fn register_one(app: &AppHandle, shortcut: Shortcut) -> Result<(), String> {
    let app_for_handler = app.clone();
    app.global_shortcut()
        .on_shortcut(shortcut, move |_app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                toggle_main_window(&app_for_handler);
            }
        })
        .map_err(|error| format!("Failed to register shortcut: {error:?}"))?;
    REGISTERED_SHORTCUTS.lock().unwrap().push(shortcut);
    Ok(())
}

fn register_configuration_without_fallback(
    app: &AppHandle,
    configuration: &ShortcutConfiguration,
) -> Result<(), String> {
    let Some(hotkey) = configuration.hotkey.as_deref() else {
        return Ok(());
    };
    let shortcut = Shortcut::from_str(&parser_hotkey(hotkey))
        .map_err(|error| format!("Invalid shortcut: {error:?}"))?;
    register_one(app, shortcut)?;
    if configuration.replace_win_v && !hotkey.eq_ignore_ascii_case(WIN_V_BRIDGE_SHORTCUT) {
        let bridge = Shortcut::from_str(&parser_hotkey(WIN_V_BRIDGE_SHORTCUT))
            .map_err(|error| format!("Invalid Win+V bridge shortcut: {error:?}"))?;
        register_bridge(app, bridge);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        parser_hotkey, startup_notice_kind, startup_recovery_delay, STARTUP_RECOVERY_MAX_DELAY,
        STARTUP_RECOVERY_WINDOW,
    };
    use std::time::Duration;

    #[test]
    fn a_missing_bridge_alone_needs_no_notice() {
        assert_eq!(startup_notice_kind("Ctrl+B", Some("Ctrl+B"), false), None);
    }

    #[test]
    fn startup_notice_names_what_the_user_lost() {
        assert_eq!(
            startup_notice_kind("Win+Alt+V", Some("Win+Ctrl+Alt+V"), false),
            Some("fallback_hotkey")
        );
        assert_eq!(
            startup_notice_kind("Win+Alt+V", Some("Win+Alt+V"), true),
            Some("win_v_disabled")
        );
        assert_eq!(
            startup_notice_kind("Win+Alt+V", None, false),
            Some("failed")
        );
    }

    #[test]
    fn startup_recovery_backs_off_then_gives_up() {
        assert_eq!(
            startup_recovery_delay(0, Duration::ZERO),
            Some(Duration::from_secs(2))
        );
        assert_eq!(
            startup_recovery_delay(1, Duration::ZERO),
            Some(Duration::from_secs(4))
        );
        assert_eq!(
            startup_recovery_delay(40, Duration::ZERO),
            Some(STARTUP_RECOVERY_MAX_DELAY)
        );
        assert_eq!(startup_recovery_delay(1, STARTUP_RECOVERY_WINDOW), None);
    }

    #[test]
    fn normalizes_windows_friendly_modifier_names() {
        assert_eq!(parser_hotkey("Win+Alt+V"), "Super+Alt+V");
        assert_eq!(parser_hotkey("Windows+Ctrl+Alt+V"), "Super+Ctrl+Alt+V");
    }

    #[test]
    fn preserves_other_shortcut_tokens() {
        assert_eq!(parser_hotkey("Ctrl+Shift+V"), "Ctrl+Shift+V");
    }
}
