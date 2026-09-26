import { useEffect, useState } from 'react';
import { getVersion } from '@tauri-apps/api/app';
import { openUrl } from '@tauri-apps/plugin-opener';
import { check } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';
import { ExternalLink, Github, Globe, Info, RefreshCw, ShieldCheck } from 'lucide-react';
import { toast } from 'sonner';
import type { Settings } from '../../types';
import { CubbyMark, PaneHeader, SettingCard, ghostButton } from './ui';

const GITHUB_URL = 'https://github.com/btsouth/cubby-clipboard';
const WEBSITE_URL = 'https://cubbyclipboard.com';
const PRIVACY_URL = 'https://cubbyclipboard.com/privacy';

export function AboutTab({ settings }: { settings: Settings }) {
  const [appVersion, setAppVersion] = useState('');
  const [checkingUpdate, setCheckingUpdate] = useState(false);

  useEffect(() => {
    getVersion().then(setAppVersion).catch(console.error);
  }, []);

  // A URL missing from the opener allowlist rejects at the Tauri boundary with
  // no visible effect, so a swallowed console.error looked exactly like a dead
  // button. Say so instead: the user can still reach the page in a browser.
  const handleOpenUrl = (url: string) => {
    openUrl(url).catch((error) => {
      console.error(`Could not open ${url}:`, error);
      toast.error(`Could not open ${url} in your browser.`);
    });
  };

  const handleCheckUpdates = async () => {
    setCheckingUpdate(true);
    try {
      const update = await check();
      if (update?.available) {
        toast(`Cubby ${update.version} is available.`, {
          duration: Infinity,
          action: {
            label: 'Update now',
            onClick: () => {
              const toastId = toast.loading('Downloading update…');
              update
                .downloadAndInstall()
                .then(() => {
                  toast.success('Update ready — restarting Cubby…', { id: toastId });
                  return relaunch();
                })
                .catch((error) => {
                  console.error('Update install failed:', error);
                  toast.error('Update failed. Please try again later.', { id: toastId });
                });
            },
          },
        });
      } else {
        toast.success("You're on the latest version.");
      }
    } catch (error) {
      console.error('Update check failed:', error);
      toast.error('Update failed. Please try again later.');
    } finally {
      setCheckingUpdate(false);
    }
  };

  return (
    <div className="space-y-7">
      <PaneHeader title={'About'} subtitle={'Version, updates, and the open-source project.'} />
      <section>
        <SettingCard>
          <div className="flex items-center gap-4 px-4 py-4">
            <CubbyMark className="h-11 w-11 flex-shrink-0" />
            <div className="min-w-0 flex-1">
              <div className="text-base font-semibold">Cubby</div>
              <div className="text-xs text-muted-foreground">{`Version ${appVersion || '…'}`}</div>
            </div>
            {settings.self_update_available !== false && (
              <button
                onClick={handleCheckUpdates}
                disabled={checkingUpdate}
                className={ghostButton}
              >
                <RefreshCw size={14} className={checkingUpdate ? 'animate-spin' : ''} />
                {'Check for updates'}
              </button>
            )}
          </div>
          {/* Portable builds have no self-update: it installs an
              NSIS package, which would split the app from the data
              folder the user carries with it. Say how to upgrade by
              hand instead of leaving a version number and no path
              forward. Store builds need nothing here — Windows
              updates them. */}
          {settings.self_update_available === false && settings.is_portable && (
            <div className="flex gap-3 px-4 py-3.5">
              <Info size={16} className="mt-0.5 flex-shrink-0 text-muted-foreground" />
              <p className="text-xs leading-relaxed text-muted-foreground">
                {
                  "The portable version does not update itself. To upgrade, download the newest portable ZIP and replace Cubby Clipboard.exe. Keep the 'data' folder and portable.txt next to it and your history, settings, and folders carry over."
                }
              </p>
            </div>
          )}
          <button
            onClick={() => handleOpenUrl(GITHUB_URL)}
            className="flex w-full items-center gap-3 px-4 py-3 text-sm transition-colors hover:bg-accent/40"
          >
            <Github size={16} className="text-muted-foreground" />
            <span className="flex-1 text-left">{'Source code on GitHub'}</span>
            <ExternalLink size={14} className="text-muted-foreground" />
          </button>
          <button
            onClick={() => handleOpenUrl(WEBSITE_URL)}
            className="flex w-full items-center gap-3 px-4 py-3 text-sm transition-colors hover:bg-accent/40"
          >
            <Globe size={16} className="text-muted-foreground" />
            <span className="flex-1 text-left">cubbyclipboard.com</span>
            <ExternalLink size={14} className="text-muted-foreground" />
          </button>
          <button
            onClick={() => handleOpenUrl(PRIVACY_URL)}
            className="flex w-full items-center gap-3 px-4 py-3 text-sm transition-colors hover:bg-accent/40"
          >
            <ShieldCheck size={16} className="text-muted-foreground" />
            <span className="flex-1 text-left">{'Privacy policy'}</span>
            <ExternalLink size={14} className="text-muted-foreground" />
          </button>
        </SettingCard>
        <div className="mt-3 flex gap-3 rounded-xl border border-border bg-card/60 p-3.5">
          <Info size={16} className="mt-0.5 flex-shrink-0 text-muted-foreground" />
          <p className="text-xs leading-relaxed text-muted-foreground">
            <span className="font-semibold text-foreground">{'Free and open source.'}</span>{' '}
            {'Cubby is GPL-3.0, a fork of PastePaw. © 2026 SouthForge AI.'}
          </p>
        </div>
      </section>
    </div>
  );
}
