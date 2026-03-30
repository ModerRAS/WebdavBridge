import { useState, useEffect } from 'react';
import type { FormEvent } from 'react';
import api from '../lib/api';

interface Config {
  upstream_url: string;
  upstream_username: string | null;
  cache_dir: string;
  metadata_db_path: string;
  rate_limit_permits: number;
  metadata_update_interval_secs: number;
  max_depth: number;
  server_bind: string;
  server_prefix: string;
  max_symlink_depth: number;
}

export default function ConfigPage() {
  const [config, setConfig] = useState<Config | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [message, setMessage] = useState('');
  const [error, setError] = useState('');

  useEffect(() => {
    api.get('/config').then((resp) => {
      setConfig(resp.data);
      setLoading(false);
    }).catch(() => {
      setError('Failed to load config');
      setLoading(false);
    });
  }, []);

  const handleSubmit = async (e: FormEvent) => {
    e.preventDefault();
    if (!config) return;
    setSaving(true);
    setMessage('');
    setError('');
    try {
      await api.put('/config', {
        upstream_url: config.upstream_url,
        upstream_username: config.upstream_username,
        rate_limit_permits: config.rate_limit_permits,
        metadata_update_interval_secs: config.metadata_update_interval_secs,
        max_depth: config.max_depth,
        server_prefix: config.server_prefix,
        max_symlink_depth: config.max_symlink_depth,
      });
      setMessage('Config saved successfully');
    } catch {
      setError('Failed to save config');
    } finally {
      setSaving(false);
    }
  };

  if (loading) return <div className="text-gray-500 dark:text-gray-400">Loading...</div>;
  if (!config) return <div className="text-red-500">{error}</div>;

  return (
    <div className="max-w-2xl mx-auto">
      <h1 className="text-2xl font-bold text-gray-900 dark:text-white mb-6">Configuration</h1>

      {message && <div className="mb-4 p-3 bg-green-100 dark:bg-green-900 text-green-700 dark:text-green-300 rounded">{message}</div>}
      {error && <div className="mb-4 p-3 bg-red-100 dark:bg-red-900 text-red-700 dark:text-red-300 rounded">{error}</div>}

      <form onSubmit={handleSubmit} className="space-y-6 bg-white dark:bg-gray-800 p-6 rounded-lg shadow">
        <div>
          <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Upstream URL</label>
          <input
            type="url"
            value={config.upstream_url}
            onChange={(e) => setConfig({ ...config, upstream_url: e.target.value })}
            className="w-full px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-md bg-white dark:bg-gray-700 text-gray-900 dark:text-white focus:ring-2 focus:ring-blue-500 focus:outline-none"
            required
          />
        </div>

        <div>
          <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Upstream Username</label>
          <input
            type="text"
            value={config.upstream_username || ''}
            onChange={(e) => setConfig({ ...config, upstream_username: e.target.value || null })}
            className="w-full px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-md bg-white dark:bg-gray-700 text-gray-900 dark:text-white focus:ring-2 focus:ring-blue-500 focus:outline-none"
          />
        </div>

        <div className="grid grid-cols-2 gap-4">
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Rate Limit Permits</label>
            <input
              type="number"
              min={1}
              value={config.rate_limit_permits}
              onChange={(e) => setConfig({ ...config, rate_limit_permits: parseInt(e.target.value) || 1 })}
              className="w-full px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-md bg-white dark:bg-gray-700 text-gray-900 dark:text-white focus:ring-2 focus:ring-blue-500 focus:outline-none"
            />
          </div>
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Update Interval (sec)</label>
            <input
              type="number"
              min={10}
              value={config.metadata_update_interval_secs}
              onChange={(e) => setConfig({ ...config, metadata_update_interval_secs: parseInt(e.target.value) || 300 })}
              className="w-full px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-md bg-white dark:bg-gray-700 text-gray-900 dark:text-white focus:ring-2 focus:ring-blue-500 focus:outline-none"
            />
          </div>
        </div>

        <div className="grid grid-cols-2 gap-4">
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Max Depth</label>
            <input
              type="number"
              min={1}
              value={config.max_depth}
              onChange={(e) => setConfig({ ...config, max_depth: parseInt(e.target.value) || 10 })}
              className="w-full px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-md bg-white dark:bg-gray-700 text-gray-900 dark:text-white focus:ring-2 focus:ring-blue-500 focus:outline-none"
            />
          </div>
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Max Symlink Depth</label>
            <input
              type="number"
              min={1}
              value={config.max_symlink_depth}
              onChange={(e) => setConfig({ ...config, max_symlink_depth: parseInt(e.target.value) || 3 })}
              className="w-full px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-md bg-white dark:bg-gray-700 text-gray-900 dark:text-white focus:ring-2 focus:ring-blue-500 focus:outline-none"
            />
          </div>
        </div>

        <div>
          <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Server Prefix</label>
          <input
            type="text"
            value={config.server_prefix}
            onChange={(e) => setConfig({ ...config, server_prefix: e.target.value })}
            className="w-full px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-md bg-white dark:bg-gray-700 text-gray-900 dark:text-white focus:ring-2 focus:ring-blue-500 focus:outline-none"
          />
        </div>

        <div className="grid grid-cols-2 gap-4">
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Cache Dir</label>
            <input type="text" value={config.cache_dir} disabled className="w-full px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-md bg-gray-100 dark:bg-gray-600 text-gray-500 dark:text-gray-400" />
          </div>
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Server Bind</label>
            <input type="text" value={config.server_bind} disabled className="w-full px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-md bg-gray-100 dark:bg-gray-600 text-gray-500 dark:text-gray-400" />
          </div>
        </div>

        <div className="flex justify-end space-x-3">
          <button type="submit" disabled={saving} className="px-4 py-2 bg-blue-600 text-white rounded-md hover:bg-blue-700 disabled:opacity-50">
            {saving ? 'Saving...' : 'Save Configuration'}
          </button>
        </div>
      </form>
    </div>
  );
}
