import type { Settings } from '../../types';
import {
  AlertTriangle,
  FolderOpen,
  Lock,
  Pause,
  Play,
  Plus,
  RefreshCw,
  ShieldCheck,
  Trash2,
  X,
} from 'lucide-react';
import { useEffect, useId, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { clsx } from 'clsx';
import { useModalDialog } from '../../hooks/useModalDialog';
import { ConfirmDialog } from '../ConfirmDialog';
import { Select } from '../ui/Select';
import { PaneHeader, Row, SectionLabel, SettingCard, Toggle, ghostButton } from './ui';
import type { UpdateSetting } from './types';

interface PrivacyTabProps {
  settings: Settings;
  updateSetting: UpdateSetting;
  onRetentionChange: (value: string, refreshStorage: () => Promise<void>) => void;
}

type BackupImportResult = {
  total: number;
  imported: number;
  duplicates: number;
  errors: string[];
  dry_run: boolean;
};

type DittoImportResult = {
  total: number;
  imported: number;
  duplicates: number;
  skipped_groups: number;
  skipped_images: number;
  skipped_empty: number;
  skipped_malformed: number;
  errors: string[];
  dry_run: boolean;
};

function dittoSkipped(result: DittoImportResult): number {
  return (
    result.skipped_groups + result.skipped_images + result.skipped_empty + result.skipped_malformed
  );
}

type OcrQueueStatus = {
  pending: number;
  processing: number;
  completed: number;
  failed: number;
  unavailable: number;
  paused: boolean;
};

type StorageUsage = {
  items: number;
  bytes: number;
};

type StorageReclaim = {
  freed_bytes: number;
  usage: StorageUsage;
};

function formatBytes(bytes: number): string {
  if (bytes <= 0) return '0 MB';
  if (bytes < 1024) return `${bytes} B`;
  const kb = bytes / 1024;
  if (kb < 1024) return `${Math.round(kb)} KB`;
  const mb = kb / 1024;
  if (mb < 1024) return `${mb < 10 ? mb.toFixed(1) : Math.round(mb)} MB`;
  return `${(mb / 1024).toFixed(1)} GB`;
}

export function PrivacyTab({ settings, updateSetting, onRetentionChange }: PrivacyTabProps) {
  const [ignoredApps, setIgnoredApps] = useState<string[]>([]);
  const [newIgnoredApp, setNewIgnoredApp] = useState('');
  const [dittoBusy, setDittoBusy] = useState(false);
  const [backupBusy, setBackupBusy] = useState(false);
  // Which passphrase prompt is open, if any.
  const [backupPrompt, setBackupPrompt] = useState<'export' | 'import' | null>(null);
  const [backupPassphrase, setBackupPassphrase] = useState('');
  const [backupPassphraseConfirm, setBackupPassphraseConfirm] = useState('');
  const backupPromptTitleId = useId();
  const backupPromptDescriptionId = useId();
  const [ocrStatus, setOcrStatus] = useState<OcrQueueStatus | null>(null);
  const [ocrActionBusy, setOcrActionBusy] = useState(false);
  const [storageUsage, setStorageUsage] = useState<StorageUsage | null>(null);
  const [reclaiming, setReclaiming] = useState(false);
  const ocrRemaining = (ocrStatus?.pending ?? 0) + (ocrStatus?.processing ?? 0);
  const ocrFailures = (ocrStatus?.failed ?? 0) + (ocrStatus?.unavailable ?? 0);
  const ocrStatusLabel = !ocrStatus
    ? 'Loading...'
    : ocrStatus.paused
      ? 'Screenshot indexing paused'
      : ocrRemaining > 0
        ? 'Indexing screenshots'
        : ocrFailures > 0
          ? 'Screenshot index needs attention'
          : 'Screenshot index ready';

  // Confirmation Dialog State
  const [confirmDialog, setConfirmDialog] = useState({
    isOpen: false,
    title: '',
    message: '',
    action: async () => {},
  });

  const loadOcrStatus = async () => {
    try {
      setOcrStatus(await invoke<OcrQueueStatus>('get_ocr_queue_status'));
    } catch (error) {
      console.error('Failed to load OCR index status:', error);
    }
  };

  const loadStorageUsage = async () => {
    try {
      setStorageUsage(await invoke<StorageUsage>('get_storage_usage'));
    } catch (error) {
      console.error('Failed to load storage usage:', error);
    }
  };

  useEffect(() => {
    invoke<string[]>('get_ignored_apps').then(setIgnoredApps).catch(console.error);
    loadOcrStatus();
    loadStorageUsage();
    const ocrStatusTimer = window.setInterval(loadOcrStatus, 3000);
    return () => window.clearInterval(ocrStatusTimer);
  }, []);

  const handleRetentionChange = (value: string) => onRetentionChange(value, loadStorageUsage);
  const handleOcrPauseToggle = async () => {
    if (!ocrStatus) return;
    setOcrActionBusy(true);
    try {
      await invoke('set_ocr_queue_paused', { paused: !ocrStatus.paused });
      await loadOcrStatus();
    } catch (error) {
      toast.error(String(error));
    } finally {
      setOcrActionBusy(false);
    }
  };

  const handleRetryOcr = async () => {
    setOcrActionBusy(true);
    try {
      const count = await invoke<number>('retry_failed_ocr');
      toast.success(`Queued ${count} images for another OCR attempt`);
      await loadOcrStatus();
    } catch (error) {
      toast.error(String(error));
    } finally {
      setOcrActionBusy(false);
    }
  };

  const handleAddIgnoredApp = async () => {
    if (!newIgnoredApp.trim()) return;
    try {
      await invoke('add_ignored_app', { appName: newIgnoredApp.trim() });
      setIgnoredApps((prev) => [...prev, newIgnoredApp.trim()].sort());
      setNewIgnoredApp('');
      toast.success(`Added ${newIgnoredApp.trim()} to ignored apps`);
    } catch (e) {
      toast.error(`Failed to add ignored app: ${e}`);
      console.error(e);
    }
  };

  const handleBrowseFile = async () => {
    try {
      const path = await invoke<string>('pick_file');
      const filename = path.split(/[\\/]/).pop() || path;
      setNewIgnoredApp(filename);
    } catch (e) {
      console.log('File picker cancelled or failed', e);
    }
  };

  const handleImportFromDitto = async () => {
    let dbPath: string;
    try {
      dbPath = await invoke<string>('pick_ditto_database');
    } catch {
      return; // picker cancelled
    }

    let preview: DittoImportResult;
    setDittoBusy(true);
    try {
      preview = await invoke<DittoImportResult>('import_from_ditto', { dbPath, dryRun: true });
    } catch (e) {
      toast.error(`Ditto import failed: ${String(e)}`);
      return;
    } finally {
      setDittoBusy(false);
    }

    if (preview.imported === 0) {
      const skipped = dittoSkipped(preview);
      if (skipped > 0) {
        toast.info(`No clips to import from Ditto; ${skipped} skipped.`);
      } else {
        toast.info(
          preview.duplicates > 0
            ? 'Everything in that Ditto database is already in Cubby.'
            : 'No importable clips were found in that Ditto database.'
        );
      }
      return;
    }

    setConfirmDialog({
      isOpen: true,
      title: 'Import from Ditto',
      message: `Import ${preview.imported} clips from Ditto? ${preview.duplicates} duplicates and ${dittoSkipped(preview)} unsupported or malformed rows will be skipped.`,
      action: async () => {
        setDittoBusy(true);
        try {
          const result = await invoke<DittoImportResult>('import_from_ditto', {
            dbPath,
            dryRun: false,
          });
          const summary = `Imported ${result.imported} clips from Ditto; ${dittoSkipped(result)} skipped`;
          if (result.errors.length > 0) {
            toast.warning(`${summary}; ${result.errors.length} errors`);
          } else {
            toast.success(summary);
          }
        } catch (e) {
          toast.error(`Ditto import failed: ${String(e)}`);
        } finally {
          setDittoBusy(false);
        }
      },
    });
  };

  const closeBackupPrompt = () => {
    setBackupPrompt(null);
    setBackupPassphrase('');
    setBackupPassphraseConfirm('');
  };
  const backupPromptRef = useModalDialog<HTMLFormElement>(closeBackupPrompt, backupPrompt !== null);

  const handleExportBackup = async (passphrase: string) => {
    let path: string;
    try {
      path = await invoke<string>('pick_backup_save_path');
    } catch {
      return; // picker cancelled
    }
    setBackupBusy(true);
    try {
      const count = await invoke<number>('export_backup', { path, passphrase });
      toast.success(
        count === 1 ? 'Exported 1 clip to an encrypted backup' : `Exported ${count} clips`
      );
    } catch (e) {
      toast.error(`Backup failed: ${String(e)}`);
    } finally {
      setBackupBusy(false);
    }
  };

  const handleImportBackup = async (passphrase: string) => {
    let path: string;
    try {
      path = await invoke<string>('pick_backup_file');
    } catch {
      return; // picker cancelled
    }

    // Dry run first, so the confirmation can say what will actually happen and
    // a wrong passphrase is caught before anything is written.
    let preview: BackupImportResult;
    setBackupBusy(true);
    try {
      preview = await invoke<BackupImportResult>('import_backup', {
        path,
        passphrase,
        dryRun: true,
      });
    } catch (e) {
      toast.error(String(e));
      return;
    } finally {
      setBackupBusy(false);
    }

    if (preview.imported === 0) {
      toast.info(
        preview.duplicates > 0
          ? 'Every clip in that backup is already in your history'
          : 'That backup has nothing to restore'
      );
      return;
    }

    setConfirmDialog({
      isOpen: true,
      title: 'Restore from backup',
      message: `Add ${preview.imported} clip${preview.imported === 1 ? '' : 's'} from this backup?${
        preview.duplicates > 0
          ? ` ${preview.duplicates} already in your history will be skipped.`
          : ''
      }`,
      action: async () => {
        setBackupBusy(true);
        try {
          const result = await invoke<BackupImportResult>('import_backup', {
            path,
            passphrase,
            dryRun: false,
          });
          if (result.errors.length > 0) {
            toast.warning(
              `Restored ${result.imported} clips; ${result.errors.length} could not be read`
            );
          } else {
            toast.success(`Restored ${result.imported} clips`);
          }
        } catch (e) {
          toast.error(String(e));
        } finally {
          setBackupBusy(false);
        }
      },
    });
  };

  const handleRemoveIgnoredApp = async (app: string) => {
    try {
      await invoke('remove_ignored_app', { appName: app });
      setIgnoredApps((prev) => prev.filter((a) => a !== app));
      toast.success(`Removed ${app} from ignored apps`);
    } catch (e) {
      toast.error(`Failed to remove ignored app: ${e}`);
      console.error(e);
    }
  };

  const handleReclaimStorage = async () => {
    setReclaiming(true);
    try {
      const result = await invoke<StorageReclaim>('reclaim_storage');
      setStorageUsage(result.usage);
      if (result.freed_bytes > 0) {
        toast.success(`Freed ${formatBytes(result.freed_bytes)}`);
      } else {
        toast.success('Already compact — nothing to reclaim');
      }
    } catch (error) {
      console.error('Failed to reclaim space:', error);
      toast.error(`Failed to reclaim space: ${error}`);
    } finally {
      setReclaiming(false);
    }
  };

  const handleRemoveDuplicates = async () => {
    try {
      const count = await invoke<number>('remove_duplicate_clips');
      toast.success(`Removed ${count} duplicate clips`);
      loadStorageUsage();
    } catch (error) {
      console.error(error);
      toast.error(`Failed to remove duplicates: ${error}`);
    }
  };

  const confirmClearHistory = () => {
    setConfirmDialog({
      isOpen: true,
      title: 'Clear History',
      message:
        'Are you sure you want to clear your ENTIRE clipboard history? This cannot be undone.',
      action: async () => {
        try {
          await invoke('clear_all_clips');
          loadStorageUsage();
          toast.success('Clipboard history cleared successfully.');
        } catch (error) {
          console.error('Failed to clear history:', error);
          toast.error(`Failed to clear history: ${error}`);
        }
      },
    });
  };

  return (
    <>
      <ConfirmDialog
        isOpen={confirmDialog.isOpen}
        title={confirmDialog.title}
        message={confirmDialog.message}
        onConfirm={async () => {
          await confirmDialog.action();
          setConfirmDialog((prev) => ({ ...prev, isOpen: false }));
        }}
        onCancel={() => setConfirmDialog((prev) => ({ ...prev, isOpen: false }))}
      />
      {backupPrompt && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-6">
          <form
            ref={backupPromptRef}
            role="dialog"
            aria-modal="true"
            aria-labelledby={backupPromptTitleId}
            aria-describedby={backupPromptDescriptionId}
            tabIndex={-1}
            className="w-full max-w-sm rounded-lg border border-border bg-card p-4 shadow-xl"
            onSubmit={(event) => {
              event.preventDefault();
              const passphrase = backupPassphrase;
              const mode = backupPrompt;
              closeBackupPrompt();
              if (mode === 'export') void handleExportBackup(passphrase);
              else void handleImportBackup(passphrase);
            }}
          >
            <h2 id={backupPromptTitleId} className="text-sm font-semibold">
              {backupPrompt === 'export' ? 'Choose a passphrase' : 'Enter the passphrase'}
            </h2>
            <p
              id={backupPromptDescriptionId}
              className="mt-1 text-xs leading-5 text-muted-foreground"
            >
              {backupPrompt === 'export'
                ? 'This passphrase encrypts the backup file. Cubby does not store it — if you lose it, the backup cannot be opened by anyone, including you.'
                : 'The passphrase this backup was created with.'}
            </p>
            <input
              type="password"
              autoFocus
              value={backupPassphrase}
              onChange={(event) => setBackupPassphrase(event.target.value)}
              placeholder="Passphrase"
              className="mt-3 w-full rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary"
            />
            {backupPrompt === 'export' && (
              <>
                <input
                  type="password"
                  value={backupPassphraseConfirm}
                  onChange={(event) => setBackupPassphraseConfirm(event.target.value)}
                  placeholder="Confirm passphrase"
                  className="mt-2 w-full rounded-md border border-border bg-background px-2.5 py-1.5 text-sm outline-none focus:border-primary"
                />
                <p className="mt-2 text-[11px] text-muted-foreground">
                  Use at least 12 characters. Cubby cannot recover a lost passphrase.
                </p>
              </>
            )}
            <div className="mt-4 flex justify-end gap-2">
              <button type="button" onClick={closeBackupPrompt} className={ghostButton}>
                {'Cancel'}
              </button>
              <button
                type="submit"
                disabled={
                  backupPrompt === 'export'
                    ? // Count characters, not UTF-16 units: the backend floor is
                      // chars().count(), so .length would pass an 11-character
                      // passphrase with one emoji and then be refused there.
                      [...backupPassphrase].length < 12 ||
                      backupPassphrase !== backupPassphraseConfirm
                    : backupPassphrase.length === 0
                }
                className="rounded-lg bg-primary px-3 py-1.5 text-xs font-medium text-white disabled:opacity-40"
              >
                {backupPrompt === 'export' ? 'Choose location…' : 'Choose file…'}
              </button>
            </div>
          </form>
        </div>
      )}
      <div className="space-y-7">
        <PaneHeader
          title={'Privacy & data'}
          subtitle={'Choose what Cubby remembers, and manage what it already has.'}
        />

        <section>
          <SectionLabel>{'Local history'}</SectionLabel>
          <SettingCard>
            <Row
              title={
                <span className="flex items-center gap-2">
                  {'Clipboard history'}
                  <span className="inline-flex items-center gap-1.5 rounded-full border border-emerald-500/25 bg-emerald-500/10 px-2 py-0.5 text-[10px] font-semibold tracking-wide text-emerald-500">
                    <span className="h-1.5 w-1.5 rounded-full bg-emerald-500" />
                    AES-256-GCM
                  </span>
                </span>
              }
              desc={"Encrypted in Cubby's database on this computer. Nothing leaves your PC."}
            />
            <Row
              title={'Skip passwords and sensitive copies'}
              desc={
                'Skip copies with explicit privacy markers, including those from password managers. Remote Desktop adds the same Windows history flags to every copy, so Cubby can still save remote copies. Add the remote client to Ignored Apps to exclude them. Turn this off to capture app-marked passwords.'
              }
              control={
                <Toggle
                  checked={settings.skip_sensitive ?? true}
                  onChange={() =>
                    updateSetting('skip_sensitive', !(settings.skip_sensitive ?? true))
                  }
                  label={'Skip passwords and sensitive copies'}
                />
              }
            />
            <Row
              title={'Skip likely secrets in text'}
              desc={
                "Don't save text that looks like API tokens, private keys, or payment card numbers, and don't relay those clips between remote sessions. Off by default. Turn it on if you'd rather keep these out of your history (the matching is conservative, but it can skip a clip you meant to keep). Large pastes are still checked: Cubby scans the first 8 KB, which is enough to catch those markers at the start of a log or key."
              }
              control={
                <Toggle
                  checked={settings.skip_likely_secrets ?? false}
                  onChange={() =>
                    updateSetting('skip_likely_secrets', !(settings.skip_likely_secrets ?? false))
                  }
                  label={'Skip likely secrets in text'}
                />
              }
            />
            <Row
              title={'Forget copies the app clears'}
              desc={
                'If something clears the clipboard soon after a copy (common for password manager browser extensions), remove that last item from history. Only password-shaped items are removed; notes, links, and pinned items stay.'
              }
              control={
                <Toggle
                  checked={settings.forget_on_clipboard_clear ?? true}
                  onChange={() =>
                    updateSetting(
                      'forget_on_clipboard_clear',
                      !(settings.forget_on_clipboard_clear ?? true)
                    )
                  }
                  label={'Forget copies the app clears'}
                />
              }
            />
            <Row
              title={'Ignore Ghost Clips'}
              desc={'Ignore temporary clipboard items'}
              control={
                <Toggle
                  checked={settings.ignore_ghost_clips}
                  onChange={() => updateSetting('ignore_ghost_clips', !settings.ignore_ghost_clips)}
                  label={'Ignore Ghost Clips'}
                />
              }
            />
          </SettingCard>
        </section>

        <section>
          <SectionLabel>{'History retention'}</SectionLabel>
          <SettingCard>
            <Row
              title={'Keep history for'}
              control={
                <div className="w-40">
                  <Select
                    ariaLabel={'Keep history for'}
                    value={String(settings.auto_delete_days ?? 30)}
                    onChange={handleRetentionChange}
                    options={[
                      { value: '7', label: '7 days' },
                      { value: '30', label: '30 days' },
                      { value: '90', label: '90 days' },
                      { value: '365', label: '1 year' },
                      { value: '0', label: 'Forever' },
                    ]}
                  />
                </div>
              }
            />
            {settings.auto_delete_days === 0 && (
              <div className="flex gap-3 px-4 py-3">
                <AlertTriangle size={16} className="mt-0.5 flex-shrink-0 text-amber-500" />
                <p className="text-xs leading-relaxed text-muted-foreground">
                  {
                    'Cubby keeps everything you copy, including screenshots, until you clear it. This can use noticeable disk space over time.'
                  }
                </p>
              </div>
            )}
            <Row
              title={'Storage used'}
              control={
                <span className="text-xs text-muted-foreground">
                  {storageUsage
                    ? `${`${storageUsage.items} items`} · ${formatBytes(storageUsage.bytes)}`
                    : '…'}
                </span>
              }
            />
            <Row
              title={'Reclaim space'}
              desc={
                'Compact the database and remove leftover files to return freed space to your disk.'
              }
              control={
                <button
                  onClick={handleReclaimStorage}
                  disabled={reclaiming}
                  className={ghostButton}
                >
                  {reclaiming ? 'Reclaiming…' : 'Reclaim'}
                </button>
              }
            />
          </SettingCard>
          <p className="ml-1 mt-2 text-xs text-muted-foreground">
            {'Pinned items are always kept.'}
          </p>
        </section>

        <section>
          <SectionLabel>{'Ignored Applications'}</SectionLabel>
          <p className="-mt-1 mb-2 ml-1 text-xs text-muted-foreground">
            {
              "Don't capture clipboard from these apps. Major password managers are added once by default; remove any you want Cubby to capture."
            }
          </p>
          <SettingCard>
            <div className="p-2">
              {ignoredApps.length === 0 ? (
                <p className="px-2 py-3 text-center text-xs text-muted-foreground">
                  {'No ignored applications'}
                </p>
              ) : (
                <div className="space-y-0.5">
                  {ignoredApps.map((app) => (
                    <div
                      key={app}
                      className="group flex items-center gap-3 rounded-lg px-2.5 py-2 hover:bg-accent/50"
                    >
                      <span className="flex h-7 w-7 flex-shrink-0 items-center justify-center rounded-md border border-border bg-accent/40 text-muted-foreground">
                        <Lock size={14} />
                      </span>
                      <span className="flex-1 font-mono text-xs">{app}</span>
                      <button
                        onClick={() => handleRemoveIgnoredApp(app)}
                        className="rounded-md p-1 text-muted-foreground opacity-0 transition hover:bg-destructive/10 hover:text-destructive group-hover:opacity-100"
                        aria-label={`Remove ${app}`}
                      >
                        <X size={14} />
                      </button>
                    </div>
                  ))}
                </div>
              )}
            </div>
            <div className="flex gap-2 px-3 py-3">
              <input
                type="text"
                value={newIgnoredApp}
                onChange={(e) => setNewIgnoredApp(e.target.value)}
                placeholder="notepad.exe"
                aria-label="Executable to ignore"
                className="flex-1 rounded-lg border border-border bg-input px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-ring"
                onKeyDown={(e) => e.key === 'Enter' && handleAddIgnoredApp()}
              />
              <button
                onClick={handleBrowseFile}
                className={ghostButton}
                title="Browse executable"
                aria-label="Browse executable"
              >
                <FolderOpen size={14} />
              </button>
              <button
                onClick={handleAddIgnoredApp}
                disabled={!newIgnoredApp.trim()}
                className={ghostButton}
              >
                <Plus size={14} />
                {'Add'}
              </button>
            </div>
          </SettingCard>
        </section>

        <section>
          <SectionLabel>{'Screenshot text search'}</SectionLabel>
          <SettingCard>
            <Row>
              <div className="flex items-start justify-between gap-4">
                <div className="min-w-0">
                  <div className="text-sm font-medium">{ocrStatusLabel}</div>
                  <p className="mt-1 text-xs leading-snug text-muted-foreground">
                    {
                      'Cubby indexes screenshot text locally in the background. Images are never uploaded.'
                    }
                  </p>
                  {ocrStatus && (
                    <p className="mt-2 text-xs text-muted-foreground">
                      {`${ocrStatus.completed} indexed · ${ocrRemaining} remaining · ${ocrFailures} need attention`}
                    </p>
                  )}
                  {!!ocrStatus?.unavailable && (
                    <p className="mt-2 text-xs text-amber-600 dark:text-amber-400">
                      {
                        'Windows OCR is unavailable. Install an OCR language pack in Windows Language settings, then retry.'
                      }
                    </p>
                  )}
                </div>
                <button
                  onClick={handleOcrPauseToggle}
                  disabled={!ocrStatus || ocrActionBusy}
                  className={clsx(ghostButton, 'flex-shrink-0')}
                >
                  {ocrStatus?.paused ? <Play size={14} /> : <Pause size={14} />}
                  {ocrStatus?.paused ? 'Resume' : 'Pause'}
                </button>
              </div>
              {!!ocrStatus && ocrFailures > 0 && (
                <button
                  onClick={handleRetryOcr}
                  disabled={ocrActionBusy}
                  className={clsx(ghostButton, 'mt-3')}
                >
                  <RefreshCw size={14} />
                  {'Retry failed images'}
                </button>
              )}
            </Row>
          </SettingCard>
        </section>

        <section>
          <SectionLabel>{'Your data'}</SectionLabel>
          <SettingCard>
            <Row
              title={'Import from Ditto'}
              desc={'Bring your clipboard history and pinned items over from a Ditto database.'}
              control={
                <button
                  onClick={handleImportFromDitto}
                  disabled={dittoBusy}
                  className={ghostButton}
                >
                  {dittoBusy ? 'Importing…' : 'Import…'}
                </button>
              }
            />
            <Row
              title="Export an encrypted backup"
              desc="Save your history to a single encrypted file. You choose a passphrase; without it the file cannot be opened, and Cubby cannot recover it for you."
              control={
                <button
                  onClick={() => setBackupPrompt('export')}
                  disabled={backupBusy}
                  className={ghostButton}
                >
                  {backupBusy ? 'Working…' : 'Export…'}
                </button>
              }
            />
            <Row
              title="Restore from a backup"
              desc="Read a backup file back in. Clips you already have are skipped, so restoring twice is safe."
              control={
                <button
                  onClick={() => setBackupPrompt('import')}
                  disabled={backupBusy}
                  className={ghostButton}
                >
                  {backupBusy ? 'Working…' : 'Restore…'}
                </button>
              }
            />
            <Row
              title={'Remove Duplicates'}
              desc={'Collapse repeated clips into a single entry.'}
              control={
                <button onClick={handleRemoveDuplicates} className={ghostButton}>
                  {'Clean up'}
                </button>
              }
            />
            <Row
              title={'Clear History'}
              desc={'Permanently delete every stored clip on this PC.'}
              control={
                <button
                  onClick={confirmClearHistory}
                  className="inline-flex items-center gap-2 rounded-lg border border-destructive/25 bg-destructive/10 px-3 py-1.5 text-xs font-medium text-destructive transition-colors hover:bg-destructive/20"
                >
                  <Trash2 size={14} />
                  {'Clear History'}
                </button>
              }
            />
          </SettingCard>
        </section>

        <div className="flex gap-3 rounded-xl border border-primary/20 bg-primary/[0.06] p-3.5">
          <ShieldCheck size={16} className="mt-0.5 flex-shrink-0 text-primary" />
          <p className="text-xs leading-relaxed text-muted-foreground">
            <span className="font-semibold text-foreground">{'No tracking.'}</span>{' '}
            {
              "Cubby's desktop app sends no product analytics, and your clipboard history never leaves this computer."
            }
          </p>
        </div>
      </div>
    </>
  );
}
