import type { Settings } from '../../types';
import { useState } from 'react';
import { useShortcutRecorder } from 'use-shortcut-recorder';
import { clsx } from 'clsx';
import { PaneHeader, Row, SectionLabel, Segmented, SettingCard, Toggle, ghostButton } from './ui';
import type { UpdateSetting } from './types';

interface GeneralTabProps {
  settings: Settings;
  updateSetting: UpdateSetting;
}

export function GeneralTab({ settings, updateSetting }: GeneralTabProps) {
  const [isRecordingMode, setIsRecordingMode] = useState(false);
  const handleThemeChange = (newTheme: string) => {
    updateSetting('theme', newTheme);
  };

  // Use use-shortcut-recorder for recording (shows current keys held in real-time)
  const {
    shortcut,
    savedShortcut,
    startRecording: startRecordingLib,
    stopRecording: stopRecordingLib,
    clearLastRecording,
  } = useShortcutRecorder({
    minModKeys: 1, // Require at least one modifier
  });

  // Start recording mode
  const handleStartRecording = () => {
    setIsRecordingMode(true);
    startRecordingLib();
  };
  // Format shortcut array into Tauri-compatible string
  const formatHotkey = (keys: string[]): string => {
    return keys
      .map((k) => {
        if (k === 'Control') return 'Ctrl';
        if (k === 'Alt') return 'Alt';
        if (k === 'Shift') return 'Shift';
        if (k === 'Meta') return 'Cmd';
        if (k.startsWith('Key')) return k.slice(3);
        if (k.startsWith('Digit')) return k.slice(5);
        return k;
      })
      .join('+');
  };

  const handleSaveHotkey = async () => {
    if (savedShortcut.length > 0) {
      const newHotkey = formatHotkey(savedShortcut);
      await updateSetting('hotkey', newHotkey);
    }
    stopRecordingLib();
    setIsRecordingMode(false);
  };

  const handleCancelRecording = () => {
    stopRecordingLib();
    clearLastRecording();
    setIsRecordingMode(false);
  };

  const windowEffectValue =
    settings.mica_effect === 'clear'
      ? 'solid'
      : settings.mica_effect === 'mica_alt' || settings.mica_effect === 'auto'
        ? 'acrylic'
        : settings.mica_effect || 'solid';

  return (
    <div className="space-y-7">
      <PaneHeader title={'General'} subtitle={'How Cubby looks and behaves on this PC.'} />

      <section>
        <SectionLabel>{'Appearance'}</SectionLabel>
        <SettingCard>
          <Row
            title={'Theme'}
            control={
              <Segmented
                value={settings.theme}
                onChange={handleThemeChange}
                options={[
                  { value: 'dark', label: 'Dark' },
                  { value: 'system', label: 'System' },
                  { value: 'light', label: 'Light' },
                ]}
              />
            }
          />
          <Row
            title={'Window Effect'}
            desc={'Surface style of the popup window.'}
            control={
              <Segmented
                value={windowEffectValue}
                onChange={(val) => updateSetting('mica_effect', val)}
                options={[
                  { value: 'solid', label: 'Solid' },
                  { value: 'mica', label: 'Mica' },
                  { value: 'acrylic', label: 'Acrylic' },
                ]}
              />
            }
          />
          <Row
            title={'History density'}
            desc={'Fit more clips, or show larger previews.'}
            control={
              <Segmented
                value={settings.density ?? 'comfortable'}
                onChange={(val) =>
                  updateSetting('density', val as NonNullable<Settings['density']>)
                }
                options={[
                  { value: 'comfortable', label: 'Comfortable' },
                  { value: 'compact', label: 'Compact' },
                ]}
              />
            }
          />
          <Row
            title={'Rounded Corners'}
            desc={'Use rounded corners for the Cubby flyout.'}
            control={
              <Toggle
                checked={settings.round_corners ?? false}
                onChange={() => updateSetting('round_corners', !(settings.round_corners ?? false))}
                label={'Rounded Corners'}
              />
            }
          />
        </SettingCard>
      </section>

      <section>
        <SectionLabel>{'Window behavior'}</SectionLabel>
        <SettingCard>
          <Row
            title={'Startup with Windows'}
            desc={
              settings.startup_unavailable_reason === 'error'
                ? 'Cubby could not read Windows startup permissions. Restart Cubby and try again.'
                : settings.startup_unavailable_reason === 'app_store'
                  ? 'Not available in the Microsoft Store version. Windows manages Store app startup permissions.'
                  : settings.is_portable
                    ? 'Not available in the portable version (it never touches the registry).'
                    : 'Automatically start when Windows boots'
            }
            control={
              <Toggle
                checked={settings.startup_with_windows}
                disabled={settings.startup_available === false}
                onChange={() =>
                  updateSetting('startup_with_windows', !settings.startup_with_windows)
                }
                label={'Startup with Windows'}
              />
            }
          />
          <Row
            title={'Float Above Taskbar'}
            desc={'Show the window on top of the taskbar'}
            control={
              <Toggle
                checked={settings.float_above_taskbar ?? true}
                onChange={() =>
                  updateSetting('float_above_taskbar', !(settings.float_above_taskbar ?? true))
                }
                label={'Float Above Taskbar'}
              />
            }
          />
        </SettingCard>
      </section>

      <section>
        <SectionLabel>{'Shortcuts'}</SectionLabel>
        <SettingCard>
          <Row title={'Open Cubby'} desc={'Shortcut to show or hide Cubby'}>
            {isRecordingMode ? (
              <div className="space-y-2">
                <div className="flex w-full items-center gap-2 rounded-lg border border-primary bg-input px-3 py-2 text-sm ring-2 ring-primary">
                  <span className="animate-pulse text-primary">
                    {shortcut.length > 0
                      ? formatHotkey(shortcut)
                      : savedShortcut.length > 0
                        ? formatHotkey(savedShortcut)
                        : 'Press keys...'}
                  </span>
                </div>
                <div className="flex gap-2">
                  <button
                    onClick={handleSaveHotkey}
                    disabled={savedShortcut.length === 0}
                    className="rounded bg-primary px-3 py-1 text-xs text-primary-foreground disabled:opacity-50"
                  >
                    {'Save'}
                  </button>
                  <button
                    onClick={handleCancelRecording}
                    className="rounded bg-muted px-3 py-1 text-xs text-muted-foreground"
                  >
                    {'Cancel'}
                  </button>
                </div>
              </div>
            ) : (
              <div className="flex items-center gap-2">
                <span className="rounded-md border border-border bg-accent/50 px-2.5 py-1 font-mono text-xs font-medium">
                  {settings.hotkey}
                </span>
                <button onClick={handleStartRecording} className={ghostButton}>
                  {'Change'}
                </button>
              </div>
            )}
          </Row>
          <Row
            title={'Replace Windows clipboard shortcut'}
            desc={
              'Cubby takes over Win+V, so Windows Clipboard History will not open. Emoji picker remains available with Win+Period.'
            }
            control={
              <Toggle
                checked={settings.replace_win_v}
                onChange={() => updateSetting('replace_win_v', !settings.replace_win_v)}
                label={'Replace Windows clipboard shortcut'}
              />
            }
          />
          <Row
            title={'Remote session paste'}
            desc={
              'Choose how Cubby handles clips selected while a supported remote-control app is focused.'
            }
          >
            <div className="grid grid-cols-2 gap-2">
              <button
                onClick={() => updateSetting('remote_paste_mode', 'copy_then_paste')}
                className={clsx(
                  'rounded-lg border px-3 py-2 text-left text-xs transition-colors',
                  settings.remote_paste_mode === 'copy_then_paste'
                    ? 'border-primary/60 bg-primary/10 text-foreground'
                    : 'border-border bg-accent/30 text-muted-foreground hover:border-primary/40'
                )}
              >
                <span className="block font-medium">{'Copy, then Ctrl+V'}</span>
                <span className="mt-1 block leading-snug">
                  {
                    'Recommended for large logs and everyday use. Cubby restores the remote app, then you press Ctrl+V.'
                  }
                </span>
              </button>
              <button
                onClick={() => updateSetting('remote_paste_mode', 'paste_as_keystrokes')}
                className={clsx(
                  'rounded-lg border px-3 py-2 text-left text-xs transition-colors',
                  settings.remote_paste_mode === 'paste_as_keystrokes'
                    ? 'border-primary/60 bg-primary/10 text-foreground'
                    : 'border-border bg-accent/30 text-muted-foreground hover:border-primary/40'
                )}
              >
                <span className="block font-medium">{'Paste as keystrokes'}</span>
                <span className="mt-1 block leading-snug">
                  {'Automatically types short text through Ninja Remote. Slow for large content.'}
                </span>
              </button>
            </div>
            {settings.replace_win_v && (
              <p className="mt-3 text-xs text-muted-foreground">
                {
                  "Replace Windows clipboard shortcut also lets your hotkey open Cubby inside remote sessions when the remote client's keyboard forwarding is off. With forwarding on, open Cubby from the tray icon."
                }
              </p>
            )}
          </Row>
          <Row
            title={'Sync clipboard between remote sessions'}
            desc={
              'Re-announce a copy from a remote session so your other remote sessions pick it up. Copies skipped as sensitive or as likely secrets are not relayed. Copy in one session, Ctrl+V in another.'
            }
            control={
              <Toggle
                checked={settings.remote_clipboard_relay !== false}
                onChange={() =>
                  updateSetting('remote_clipboard_relay', settings.remote_clipboard_relay === false)
                }
                label={'Sync clipboard between remote sessions'}
              />
            }
          />
        </SettingCard>
      </section>
    </div>
  );
}
