import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { invoke } from '@tauri-apps/api/core';
import { emit } from '@tauri-apps/api/event';
import { openUrl } from '@tauri-apps/plugin-opener';
import { toast } from 'sonner';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { FolderItem, Settings } from '../types';
import { SettingsPanel } from './SettingsPanel';

vi.mock('@tauri-apps/api/event', () => ({ emit: vi.fn() }));
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ startDragging: vi.fn() }),
}));
vi.mock('@tauri-apps/api/app', () => ({ getVersion: vi.fn(async () => '9.8.7') }));
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: vi.fn() }));
vi.mock('@tauri-apps/plugin-updater', () => ({ check: vi.fn() }));
vi.mock('@tauri-apps/plugin-process', () => ({ relaunch: vi.fn() }));
vi.mock('sonner', () => ({
  toast: Object.assign(vi.fn(), {
    success: vi.fn(),
    error: vi.fn(),
    info: vi.fn(),
    warning: vi.fn(),
    loading: vi.fn(),
  }),
}));

const baseSettings: Settings = {
  max_items: 0,
  auto_delete_days: 30,
  startup_with_windows: false,
  show_in_taskbar: false,
  hotkey: 'Ctrl+Shift+V',
  replace_win_v: false,
  theme: 'dark',
  mica_effect: 'mica',
  round_corners: false,
  remote_paste_mode: 'copy_then_paste',
  ignore_ghost_clips: false,
};

let folders: FolderItem[];
let saves: { settings: Settings; changedKeys: string[] }[];
let releaseFirstSave: (() => void) | null;

function folder(id: string, name: string, itemCount: number, isSystem = false): FolderItem {
  return { id, name, icon: null, color: null, is_system: isSystem, item_count: itemCount };
}

beforeEach(() => {
  folders = [folder('sys', 'Pinned', 4, true), folder('f1', 'Work', 3)];
  saves = [];
  releaseFirstSave = null;
  vi.mocked(invoke).mockImplementation(async (command: string, args?: unknown) => {
    const payload = args as Record<string, unknown> | undefined;
    switch (command) {
      case 'get_folders':
        return folders.map((f) => ({ ...f }));
      case 'get_ignored_apps':
        return [];
      case 'get_ocr_queue_status':
        return {
          pending: 0,
          processing: 0,
          completed: 0,
          failed: 0,
          unavailable: 0,
          paused: false,
        };
      case 'get_storage_usage':
        return { items: 0, bytes: 0 };
      case 'create_folder':
        folders.push(folder('f2', payload?.name as string, 0));
        return null;
      case 'rename_folder':
        folders = folders.map((f) =>
          f.id === payload?.id ? { ...f, name: payload?.name as string } : f
        );
        return null;
      case 'delete_folder':
        folders = folders.filter((f) => f.id !== payload?.id);
        return null;
      case 'save_settings': {
        saves.push(payload as { settings: Settings; changedKeys: string[] });
        if (saves.length === 1) {
          await new Promise<void>((resolve) => {
            releaseFirstSave = resolve;
          });
        }
        return null;
      }
      default:
        return null;
    }
  });
});

afterEach(cleanup);

function renderPanel(settings: Partial<Settings> = {}) {
  return render(<SettingsPanel settings={{ ...baseSettings, ...settings }} onClose={vi.fn()} />);
}

function openTab(name: string) {
  fireEvent.click(screen.getByRole('tab', { name }));
}

describe('SettingsPanel shell', () => {
  it('opens on General and moves between tabs with the keyboard', () => {
    renderPanel();
    const general = screen.getByRole('tab', { name: 'General' });
    expect(general).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByRole('heading', { name: 'General' })).toBeInTheDocument();

    fireEvent.keyDown(general, { key: 'ArrowDown' });
    expect(screen.getByRole('tab', { name: 'Privacy' })).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByRole('tab', { name: 'Privacy' })).toHaveFocus();

    fireEvent.keyDown(screen.getByRole('tab', { name: 'Privacy' }), { key: 'End' });
    expect(screen.getByRole('heading', { name: 'About' })).toBeInTheDocument();

    fireEvent.keyDown(screen.getByRole('tab', { name: 'About' }), { key: 'ArrowDown' });
    expect(screen.getByRole('tab', { name: 'General' })).toHaveAttribute('aria-selected', 'true');
  });

  it('serializes saves so a later change includes the earlier one', async () => {
    renderPanel();
    fireEvent.click(screen.getByRole('switch', { name: 'Rounded Corners' }));
    fireEvent.click(screen.getByRole('switch', { name: 'Float Above Taskbar' }));

    await waitFor(() => expect(saves).toHaveLength(1));
    expect(saves[0].changedKeys).toEqual(['round_corners']);
    expect(saves[0].settings.round_corners).toBe(true);

    await act(async () => releaseFirstSave?.());
    await waitFor(() => expect(saves).toHaveLength(2));
    expect(saves[1].changedKeys).toEqual(['float_above_taskbar']);
    expect(saves[1].settings.round_corners).toBe(true);
    expect(saves[1].settings.float_above_taskbar).toBe(false);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('refresh_window'));
    expect(emit).toHaveBeenCalledWith(
      'settings-changed',
      expect.objectContaining({ round_corners: true, float_above_taskbar: false })
    );
  });
});

describe('SettingsPanel Folders tab', () => {
  it('lists only custom folders with their item counts', async () => {
    renderPanel();
    openTab('Folders');
    expect(await screen.findByText('Work')).toBeInTheDocument();
    expect(screen.getByText('3 items')).toBeInTheDocument();
    expect(screen.queryByText('Pinned')).not.toBeInTheDocument();
  });

  it('creates, renames and deletes folders', async () => {
    renderPanel();
    openTab('Folders');
    await screen.findByText('Work');

    const name = screen.getByPlaceholderText('New Folder Name');
    fireEvent.change(name, { target: { value: '  Recipes  ' } });
    fireEvent.keyDown(name, { key: 'Enter' });
    expect(await screen.findByText('Recipes')).toBeInTheDocument();
    expect(invoke).toHaveBeenCalledWith('create_folder', {
      name: 'Recipes',
      icon: null,
      color: null,
    });
    expect(name).toHaveValue('');
    expect(toast.success).toHaveBeenCalledWith('Folder created');

    const workRow = screen.getByText('Work').closest('div') as HTMLElement;
    fireEvent.click(within(workRow).getByTitle('Rename'));
    const rename = screen.getByDisplayValue('Work');
    fireEvent.change(rename, { target: { value: 'Office' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    expect(await screen.findByText('Office')).toBeInTheDocument();
    expect(invoke).toHaveBeenCalledWith('rename_folder', { id: 'f1', name: 'Office' });

    const officeRow = screen.getByText('Office').closest('div') as HTMLElement;
    fireEvent.click(within(officeRow).getByTitle('Delete'));
    await waitFor(() => expect(screen.queryByText('Office')).not.toBeInTheDocument());
    expect(invoke).toHaveBeenCalledWith('delete_folder', { id: 'f1' });
  });

  it('abandons a rename on Escape without saving', async () => {
    renderPanel();
    openTab('Folders');
    const workRow = (await screen.findByText('Work')).closest('div') as HTMLElement;
    fireEvent.click(within(workRow).getByTitle('Rename'));
    fireEvent.keyDown(screen.getByDisplayValue('Work'), { key: 'Escape' });
    expect(screen.getByText('Work')).toBeInTheDocument();
    expect(invoke).not.toHaveBeenCalledWith('rename_folder', expect.anything());
  });
});

describe('SettingsPanel About tab', () => {
  it('shows the app version and opens project links', async () => {
    vi.mocked(openUrl).mockResolvedValue(undefined);
    renderPanel();
    openTab('About');
    expect(await screen.findByText('Version 9.8.7')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Check for updates/ })).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: /Source code on GitHub/ }));
    fireEvent.click(screen.getByRole('button', { name: /cubbyclipboard\.com/ }));
    fireEvent.click(screen.getByRole('button', { name: /Privacy policy/ }));
    expect(vi.mocked(openUrl).mock.calls.map(([url]) => url)).toEqual([
      'https://github.com/btsouth/cubby-clipboard',
      'https://cubbyclipboard.com',
      'https://cubbyclipboard.com/privacy',
    ]);
  });

  it('says so when a link cannot be opened', async () => {
    vi.mocked(openUrl).mockRejectedValue(new Error('not allowed'));
    vi.spyOn(console, 'error').mockImplementation(() => {});
    renderPanel();
    openTab('About');
    fireEvent.click(screen.getByRole('button', { name: /Privacy policy/ }));
    await waitFor(() =>
      expect(toast.error).toHaveBeenCalledWith(
        'Could not open https://cubbyclipboard.com/privacy in your browser.'
      )
    );
  });

  it('replaces the update button with upgrade steps in the portable build', () => {
    renderPanel({ self_update_available: false, is_portable: true });
    openTab('About');
    expect(screen.queryByRole('button', { name: /Check for updates/ })).not.toBeInTheDocument();
    expect(screen.getByText(/portable version does not update itself/)).toBeInTheDocument();
  });

  it('shows neither in a Store build', () => {
    renderPanel({ self_update_available: false, is_portable: false });
    openTab('About');
    expect(screen.queryByRole('button', { name: /Check for updates/ })).not.toBeInTheDocument();
    expect(screen.queryByText(/portable version does not update itself/)).not.toBeInTheDocument();
  });
});
