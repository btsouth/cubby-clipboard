import type { Settings } from '../types';
import {
  X,
  Settings as SettingsIcon,
  Folder as FolderIcon,
  ShieldCheck,
  Info,
  FlaskConical,
} from 'lucide-react';
import { useState, useRef, type ReactNode } from 'react';
import { useTheme } from '../hooks/useTheme';
import { invoke } from '@tauri-apps/api/core';
import { emit } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { toast } from 'sonner';
import { AboutTab } from './settings/AboutTab';
import { FoldersTab } from './settings/FoldersTab';
import { GeneralTab } from './settings/GeneralTab';
import { PrivacyTab } from './settings/PrivacyTab';
import { CubbyMark } from './settings/ui';
import { clsx } from 'clsx';

interface SettingsPanelProps {
  settings: Settings;
  onClose: () => void;
}

type Tab = 'general' | 'privacy' | 'folders' | 'about';

export function SettingsPanel({ settings: initialSettings, onClose }: SettingsPanelProps) {
  const [activeTab, setActiveTab] = useState<Tab>('general');
  const [settings, setSettings] = useState<Settings>(initialSettings);
  const settingsRef = useRef<Settings>(initialSettings);
  const settingsSaveQueue = useRef<Promise<void>>(Promise.resolve());
  // Apply theme immediately when settings.theme changes
  useTheme(settings.theme);

  // Generic handler for immediate settings updates
  const updateSettings = (updates: Partial<Settings>) => {
    settingsSaveQueue.current = settingsSaveQueue.current
      .catch(() => undefined)
      .then(async () => {
        const newSettings = { ...settingsRef.current, ...updates };
        try {
          await invoke('save_settings', {
            settings: newSettings,
            changedKeys: Object.keys(updates),
          });
          settingsRef.current = newSettings;
          setSettings(newSettings);
          await emit('settings-changed', newSettings);
          if ('float_above_taskbar' in updates) {
            await invoke('refresh_window');
          }

          const keys = Object.keys(updates);
          if (keys.length === 1) {
            const key = keys[0] as keyof Settings;
            const value = updates[key];
            if (key !== 'theme') {
              const label = key
                .split('_')
                .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
                .join(' ');
              if (typeof value === 'boolean') {
                toast.success(`${label} was ${value ? 'enabled' : 'disabled'}`);
              } else {
                toast.success(`${label} updated`);
              }
            }
          } else if (keys.length > 1) {
            toast.success('Settings updated');
          }
        } catch (error) {
          console.error(`Failed to save settings:`, error);
          toast.error(String(error));
        }
      });

    return settingsSaveQueue.current;
  };

  const updateSetting = <K extends keyof Settings>(key: K, value: Settings[K]) => {
    updateSettings({ [key]: value });
  };
  const retentionGenRef = useRef(0);
  const handleRetentionChange = (value: string, refreshStorage: () => Promise<void>) => {
    const updates: Partial<Settings> = { auto_delete_days: Number(value), max_items: 0 };
    const generation = ++retentionGenRef.current;
    settingsSaveQueue.current = settingsSaveQueue.current
      .catch(() => undefined)
      .then(async () => {
        const newSettings = { ...settingsRef.current, ...updates };
        try {
          await invoke('save_settings', {
            settings: newSettings,
            changedKeys: Object.keys(updates),
          });
          settingsRef.current = newSettings;
          setSettings(newSettings);
          await emit('settings-changed', newSettings);
          // Prune only for the latest selection, and only after its save has
          // persisted, so a rapid change can't prune with an intermediate value.
          if (generation === retentionGenRef.current) {
            await invoke('apply_retention');
          }
          await refreshStorage();
          toast.success('Settings updated');
        } catch (error) {
          console.error('Failed to update retention:', error);
          toast.error(String(error));
        }
      });
    return settingsSaveQueue.current;
  };
  const tabs: { id: Tab; label: string; icon: ReactNode }[] = [
    { id: 'general', label: 'General', icon: <SettingsIcon size={17} /> },
    { id: 'privacy', label: 'Privacy', icon: <ShieldCheck size={17} /> },
    { id: 'folders', label: 'Folders', icon: <FolderIcon size={17} /> },
    { id: 'about', label: 'About', icon: <Info size={17} /> },
  ];

  // Roving tabindex: only the selected tab is in the Tab order, and Up/Down move
  // between tabs. That is the expected tab-widget behaviour, and without it a
  // keyboard user has to Tab through every tab to reach the panel.
  const tabRefs = useRef<Partial<Record<Tab, HTMLButtonElement | null>>>({});

  const focusTabAt = (index: number) => {
    const next = tabs[(index + tabs.length) % tabs.length];
    setActiveTab(next.id);
    tabRefs.current[next.id]?.focus();
  };

  const handleTabKeyDown = (event: React.KeyboardEvent, index: number) => {
    switch (event.key) {
      case 'ArrowDown':
      case 'ArrowRight':
        event.preventDefault();
        focusTabAt(index + 1);
        break;
      case 'ArrowUp':
      case 'ArrowLeft':
        event.preventDefault();
        focusTabAt(index - 1);
        break;
      case 'Home':
        event.preventDefault();
        focusTabAt(0);
        break;
      case 'End':
        event.preventDefault();
        focusTabAt(tabs.length - 1);
        break;
    }
  };
  return (
    <>
      <div className="flex h-full select-none flex-col bg-background text-foreground">
        {/* Title bar */}
        <div
          className="flex items-center justify-between border-b border-border px-4 py-3"
          onMouseDown={(e) => {
            if (e.button === 0) {
              getCurrentWindow().startDragging();
            }
          }}
        >
          <div className="flex items-center gap-2.5">
            <CubbyMark className="h-[18px] w-[18px]" />
            <span className="text-sm font-semibold">{'Settings'}</span>
          </div>
          <button
            onClick={onClose}
            className="icon-button"
            onMouseDown={(e) => e.stopPropagation()}
            aria-label="Close settings"
          >
            <X size={18} />
          </button>
        </div>

        <div className="flex flex-1 overflow-hidden">
          {/* Sidebar */}
          <div className="flex w-[188px] flex-shrink-0 flex-col border-r border-border bg-card/40 p-2.5">
            <div
              role="tablist"
              aria-orientation="vertical"
              aria-label={'Settings'}
              className="flex flex-col gap-0.5"
            >
              {tabs.map((tab, index) => (
                <button
                  key={tab.id}
                  role="tab"
                  id={`settings-tab-${tab.id}`}
                  aria-selected={activeTab === tab.id}
                  aria-controls="settings-tabpanel"
                  tabIndex={activeTab === tab.id ? 0 : -1}
                  ref={(element) => {
                    tabRefs.current[tab.id] = element;
                  }}
                  onKeyDown={(event) => handleTabKeyDown(event, index)}
                  onClick={() => setActiveTab(tab.id)}
                  className={clsx(
                    'flex items-center gap-3 rounded-lg px-3 py-2 text-[13.5px] font-medium transition-colors',
                    activeTab === tab.id
                      ? 'bg-primary/10 text-foreground'
                      : 'text-muted-foreground hover:bg-accent/60 hover:text-foreground'
                  )}
                >
                  <span className={activeTab === tab.id ? 'text-primary' : ''}>{tab.icon}</span>
                  {tab.label}
                </button>
              ))}
            </div>
            <div className="mt-auto px-3 pt-3 text-[11px] leading-relaxed text-muted-foreground/70">
              {'Encrypted and local-only. No account required.'}
            </div>
          </div>

          {/* Content Area */}
          <div
            role="tabpanel"
            id="settings-tabpanel"
            aria-labelledby={`settings-tab-${activeTab}`}
            tabIndex={0}
            className="flex-1 overflow-y-auto px-7 py-6"
          >
            <div className="mx-auto max-w-2xl">
              <div hidden={activeTab !== 'general'}>
                <GeneralTab settings={settings} updateSetting={updateSetting} />
              </div>

              <div hidden={activeTab !== 'privacy'}>
                <PrivacyTab
                  settings={settings}
                  updateSetting={updateSetting}
                  onRetentionChange={handleRetentionChange}
                />
              </div>

              <div hidden={activeTab !== 'folders'}>
                <FoldersTab />
              </div>

              <div hidden={activeTab !== 'about'}>
                <AboutTab settings={settings} />
              </div>
            </div>
          </div>
        </div>

        {/* Debug Tools — dev build only */}
        {import.meta.env.DEV && (
          <div className="border-t border-border px-4 py-3">
            <p className="mb-2 text-xs font-medium text-muted-foreground">Debug</p>
            <div className="flex gap-2">
              <button
                onClick={() => emit('load-demo-data')}
                className="flex items-center gap-2 rounded-md border border-dashed border-border px-3 py-1.5 text-xs text-muted-foreground transition-colors hover:border-primary hover:text-primary"
              >
                <FlaskConical size={12} />
                Load 20 demo clips
              </button>
              <button
                onClick={() => emit('restore-actual-data')}
                className="flex items-center gap-2 rounded-md border border-dashed border-border px-3 py-1.5 text-xs text-muted-foreground transition-colors hover:border-destructive hover:text-destructive"
              >
                Restore actual data
              </button>
            </div>
          </div>
        )}
      </div>
    </>
  );
}
