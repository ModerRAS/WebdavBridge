import { useState, useEffect, useCallback } from 'react';
import { useSearchParams } from 'react-router-dom';
import api from '../lib/api';

interface FileEntry {
  name: string;
  path: string;
  is_dir: boolean;
  size: number;
  content_type: string | null;
  modified: string | null;
  is_symlink: boolean;
  symlink_target: string | null;
  has_local_override: boolean;
}

function formatSize(bytes: number): string {
  if (bytes === 0) return '0 B';
  const k = 1024;
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i];
}

export default function FilesPage() {
  const [searchParams, setSearchParams] = useSearchParams();
  const currentPath = searchParams.get('path') || '/';
  const [entries, setEntries] = useState<FileEntry[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');

  const fetchFiles = useCallback(async () => {
    setLoading(true);
    setError('');
    try {
      const resp = await api.get(`/files${currentPath}`);
      setEntries(resp.data.entries || []);
      setTotal(resp.data.total || 0);
    } catch (e: unknown) {
      const err = e as { response?: { data?: { error?: string } } };
      setError(err?.response?.data?.error || 'Failed to load files');
    } finally {
      setLoading(false);
    }
  }, [currentPath]);

  useEffect(() => { fetchFiles(); }, [fetchFiles]);

  const navigateTo = (path: string) => {
    setSearchParams({ path });
  };

  const goUp = () => {
    const parts = currentPath.split('/').filter(Boolean);
    parts.pop();
    navigateTo('/' + parts.join('/'));
  };

  const handleDelete = async (path: string) => {
    if (!confirm(`Delete ${path}?`)) return;
    try {
      await api.delete(`/files${path}`);
      fetchFiles();
    } catch {
      alert('Delete failed');
    }
  };

  const handleDownload = async (path: string, name: string) => {
    try {
      const resp = await api.get(`/files${path}/download`, { responseType: 'blob' });
      const url = window.URL.createObjectURL(new Blob([resp.data]));
      const a = document.createElement('a');
      a.href = url;
      a.download = name;
      a.click();
      window.URL.revokeObjectURL(url);
    } catch {
      alert('Download failed');
    }
  };

  const handleUpload = async () => {
    const input = document.createElement('input');
    input.type = 'file';
    input.onchange = async () => {
      const file = input.files?.[0];
      if (!file) return;
      try {
        const path = currentPath.endsWith('/') ? currentPath : currentPath + '/';
        await api.put(`/files${path}${file.name}`, file, {
          headers: { 'Content-Type': 'application/octet-stream' },
        });
        fetchFiles();
      } catch {
        alert('Upload failed');
      }
    };
    input.click();
  };

  const breadcrumbs = currentPath.split('/').filter(Boolean);

  return (
    <div>
      <div className="flex items-center justify-between mb-4">
        <div className="flex items-center space-x-2 text-sm">
          <button onClick={() => navigateTo('/')} className="text-blue-600 dark:text-blue-400 hover:underline">/</button>
          {breadcrumbs.map((part, i) => (
            <span key={i} className="flex items-center">
              <span className="mx-1 text-gray-400">/</span>
              <button
                onClick={() => navigateTo('/' + breadcrumbs.slice(0, i + 1).join('/'))}
                className="text-blue-600 dark:text-blue-400 hover:underline"
              >
                {part}
              </button>
            </span>
          ))}
        </div>
        <div className="flex space-x-2">
          <button onClick={fetchFiles} className="px-3 py-1 text-sm bg-gray-200 dark:bg-gray-700 rounded hover:bg-gray-300 dark:hover:bg-gray-600 text-gray-800 dark:text-gray-200">Refresh</button>
          <button onClick={handleUpload} className="px-3 py-1 text-sm bg-blue-600 text-white rounded hover:bg-blue-700">Upload</button>
        </div>
      </div>

      {error && <div className="text-red-500 mb-4">{error}</div>}
      {loading && <div className="text-gray-500 dark:text-gray-400">Loading...</div>}

      <div className="bg-white dark:bg-gray-800 rounded-lg shadow overflow-hidden">
        <table className="min-w-full divide-y divide-gray-200 dark:divide-gray-700">
          <thead className="bg-gray-50 dark:bg-gray-700">
            <tr>
              <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-300 uppercase tracking-wider">Name</th>
              <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-300 uppercase tracking-wider">Size</th>
              <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-300 uppercase tracking-wider">Modified</th>
              <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-300 uppercase tracking-wider">Type</th>
              <th className="px-6 py-3 text-right text-xs font-medium text-gray-500 dark:text-gray-300 uppercase tracking-wider">Actions</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-gray-200 dark:divide-gray-700">
            {currentPath !== '/' && (
              <tr className="hover:bg-gray-50 dark:hover:bg-gray-700 cursor-pointer" onClick={goUp}>
                <td className="px-6 py-4 text-sm text-gray-900 dark:text-white">..</td>
                <td colSpan={4}></td>
              </tr>
            )}
            {entries.map((entry) => (
              <tr key={entry.path} className="hover:bg-gray-50 dark:hover:bg-gray-700">
                <td className="px-6 py-4 text-sm">
                  {entry.is_dir ? (
                    <button onClick={() => navigateTo(entry.path)} className="text-blue-600 dark:text-blue-400 hover:underline flex items-center">
                      📁 {entry.name}
                    </button>
                  ) : (
                    <span className="text-gray-900 dark:text-white flex items-center">
                      {entry.is_symlink ? '🔗' : '📄'} {entry.name}
                      {entry.has_local_override && <span className="ml-1 text-xs text-orange-500">(override)</span>}
                    </span>
                  )}
                  {entry.is_symlink && entry.symlink_target && (
                    <span className="text-xs text-gray-500 dark:text-gray-400 ml-2">→ {entry.symlink_target}</span>
                  )}
                </td>
                <td className="px-6 py-4 text-sm text-gray-500 dark:text-gray-400">{entry.is_dir ? '-' : formatSize(entry.size)}</td>
                <td className="px-6 py-4 text-sm text-gray-500 dark:text-gray-400">{entry.modified ? new Date(entry.modified).toLocaleString() : '-'}</td>
                <td className="px-6 py-4 text-sm text-gray-500 dark:text-gray-400">{entry.content_type || (entry.is_dir ? 'directory' : '-')}</td>
                <td className="px-6 py-4 text-sm text-right space-x-2">
                  {!entry.is_dir && (
                    <button onClick={() => handleDownload(entry.path, entry.name)} className="text-blue-600 dark:text-blue-400 hover:underline text-xs">Download</button>
                  )}
                  <button onClick={() => handleDelete(entry.path)} className="text-red-600 dark:text-red-400 hover:underline text-xs">Delete</button>
                </td>
              </tr>
            ))}
            {entries.length === 0 && !loading && (
              <tr><td colSpan={5} className="px-6 py-4 text-center text-gray-500 dark:text-gray-400">Empty directory</td></tr>
            )}
          </tbody>
        </table>
        {total > 0 && <div className="px-6 py-3 text-sm text-gray-500 dark:text-gray-400 bg-gray-50 dark:bg-gray-700">{total} items</div>}
      </div>
    </div>
  );
}
