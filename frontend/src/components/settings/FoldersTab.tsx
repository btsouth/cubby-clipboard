import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Folder as FolderIcon, MoreHorizontal, Plus, Trash2 } from 'lucide-react';
import { toast } from 'sonner';
import type { FolderItem } from '../../types';
import { PaneHeader, SectionLabel, SettingCard, ghostButton } from './ui';

export function FoldersTab() {
  const [folders, setFolders] = useState<FolderItem[]>([]);
  const [newFolderName, setNewFolderName] = useState('');
  const [editingFolderId, setEditingFolderId] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState('');

  const loadFolders = async () => {
    try {
      const data = await invoke<FolderItem[]>('get_folders');
      setFolders(data);
    } catch (error) {
      console.error('Failed to load folders:', error);
    }
  };

  useEffect(() => {
    loadFolders();
  }, []);

  const handleCreateFolder = async () => {
    if (!newFolderName.trim()) return;
    try {
      await invoke('create_folder', { name: newFolderName.trim(), icon: null, color: null });
      setNewFolderName('');
      await loadFolders();
      toast.success('Folder created');
    } catch (e) {
      toast.error(`Failed to create folder: ${e}`);
    }
  };

  const handleDeleteFolder = async (id: string) => {
    try {
      await invoke('delete_folder', { id });
      await loadFolders();
      toast.success('Folder deleted');
    } catch (e) {
      toast.error(`Failed to delete folder: ${e}`);
    }
  };

  const startRenameFolder = (folder: FolderItem) => {
    setEditingFolderId(folder.id);
    setRenameValue(folder.name);
  };

  const saveRenameFolder = async () => {
    if (!editingFolderId || !renameValue.trim()) return;
    try {
      await invoke('rename_folder', { id: editingFolderId, name: renameValue.trim() });
      setEditingFolderId(null);
      setRenameValue('');
      await loadFolders();
      toast.success('Folder renamed');
    } catch (e) {
      toast.error(`Failed to rename folder: ${e}`);
    }
  };

  const customFolders = folders.filter((folder) => !folder.is_system);

  return (
    <div className="space-y-7">
      <PaneHeader title={'Folders'} subtitle={'Group pinned clips into folders you can jump to.'} />
      <section>
        <SectionLabel>{'Manage Folders'}</SectionLabel>
        <SettingCard>
          <div className="p-2">
            {customFolders.length === 0 ? (
              <p className="px-2 py-3 text-center text-xs text-muted-foreground">
                {'No custom folders created.'}
              </p>
            ) : (
              <div className="space-y-0.5">
                {customFolders.map((folder) => (
                  <div
                    key={folder.id}
                    className="group flex items-center gap-3 rounded-lg px-2.5 py-2 hover:bg-accent/50"
                  >
                    {editingFolderId === folder.id ? (
                      <div className="flex flex-1 items-center gap-2">
                        <input
                          type="text"
                          value={renameValue}
                          onChange={(e) => setRenameValue(e.target.value)}
                          className="flex-1 rounded-md border border-input bg-background px-2 py-1 text-sm"
                          autoFocus
                          onKeyDown={(e) => {
                            if (e.key === 'Enter') saveRenameFolder();
                            if (e.key === 'Escape') setEditingFolderId(null);
                          }}
                        />
                        <button
                          onClick={saveRenameFolder}
                          className="text-xs text-primary hover:underline"
                        >
                          {'Save'}
                        </button>
                        <button
                          onClick={() => setEditingFolderId(null)}
                          className="text-xs text-muted-foreground hover:underline"
                        >
                          {'Cancel'}
                        </button>
                      </div>
                    ) : (
                      <>
                        <span className="flex h-7 w-7 flex-shrink-0 items-center justify-center rounded-md border border-border bg-accent/40 text-primary">
                          <FolderIcon size={14} />
                        </span>
                        <span className="flex-1 text-sm font-medium">
                          {folder.name}
                          <span className="ml-2 text-xs font-normal text-muted-foreground">
                            {`${folder.item_count} items`}
                          </span>
                        </span>
                        <button
                          onClick={() => startRenameFolder(folder)}
                          className="rounded-md p-1 text-muted-foreground opacity-0 transition hover:bg-accent hover:text-foreground group-hover:opacity-100"
                          title="Rename"
                        >
                          <MoreHorizontal size={14} />
                        </button>
                        <button
                          onClick={() => handleDeleteFolder(folder.id)}
                          className="rounded-md p-1 text-muted-foreground opacity-0 transition hover:bg-destructive/10 hover:text-destructive group-hover:opacity-100"
                          title="Delete"
                        >
                          <Trash2 size={14} />
                        </button>
                      </>
                    )}
                  </div>
                ))}
              </div>
            )}
          </div>
          <div className="flex gap-2 px-3 py-3">
            <input
              type="text"
              value={newFolderName}
              onChange={(e) => setNewFolderName(e.target.value)}
              placeholder={'New Folder Name'}
              className="flex-1 rounded-lg border border-border bg-input px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-ring"
              onKeyDown={(e) => e.key === 'Enter' && handleCreateFolder()}
            />
            <button
              onClick={handleCreateFolder}
              disabled={!newFolderName.trim()}
              className={ghostButton}
            >
              <Plus size={14} />
              {'Add'}
            </button>
          </div>
        </SettingCard>
      </section>
    </div>
  );
}
