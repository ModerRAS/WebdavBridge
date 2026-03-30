import { useState, useEffect } from 'react';
import api from '../lib/api';
import { useWebSocket } from '../lib/useWebSocket';

interface Status {
  server: { uptime_secs: number; version: string; upstream_url: string };
  cache: { metadata_entries: number; symlink_count: number };
}

interface Stats {
  total_entries: number;
  directories: number;
  files: number;
  symlinks: number;
  local_overrides: number;
  total_size_bytes: number;
}

function formatSize(bytes: number): string {
  if (bytes === 0) return '0 B';
  const k = 1024;
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i];
}

export default function StatusPage() {
  const [status, setStatus] = useState<Status | null>(null);
  const [stats, setStats] = useState<Stats | null>(null);
  const [loading, setLoading] = useState(true);
  const { events, connected } = useWebSocket();

  const fetchStatus = async () => {
    try {
      const [statusResp, statsResp] = await Promise.all([
        api.get('/status'),
        api.get('/status/stats'),
      ]);
      setStatus(statusResp.data);
      setStats(statsResp.data);
    } catch { /* ignore */ }
    setLoading(false);
  };

  useEffect(() => { fetchStatus(); }, []);

  if (loading) return <div className="text-gray-500 dark:text-gray-400">Loading...</div>;

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold text-gray-900 dark:text-white">System Status</h1>
        <button onClick={fetchStatus} className="px-3 py-1 text-sm bg-gray-200 dark:bg-gray-700 rounded hover:bg-gray-300 dark:hover:bg-gray-600 text-gray-800 dark:text-gray-200">Refresh</button>
      </div>

      {/* Connection Status */}
      <div className="bg-white dark:bg-gray-800 p-6 rounded-lg shadow">
        <h2 className="text-lg font-semibold text-gray-900 dark:text-white mb-4">Connection</h2>
        <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
          <div>
            <div className="text-sm text-gray-500 dark:text-gray-400">WebSocket</div>
            <div className={`font-medium ${connected ? 'text-green-600 dark:text-green-400' : 'text-red-600 dark:text-red-400'}`}>
              {connected ? 'Connected' : 'Disconnected'}
            </div>
          </div>
          <div>
            <div className="text-sm text-gray-500 dark:text-gray-400">Version</div>
            <div className="font-medium text-gray-900 dark:text-white">{status?.server.version || '-'}</div>
          </div>
          <div>
            <div className="text-sm text-gray-500 dark:text-gray-400">Upstream</div>
            <div className="font-medium text-gray-900 dark:text-white text-sm truncate">{status?.server.upstream_url || '-'}</div>
          </div>
        </div>
      </div>

      {/* Cache Stats */}
      {stats && (
        <div className="bg-white dark:bg-gray-800 p-6 rounded-lg shadow">
          <h2 className="text-lg font-semibold text-gray-900 dark:text-white mb-4">Cache Statistics</h2>
          <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-6 gap-4">
            <div className="text-center p-3 bg-gray-50 dark:bg-gray-700 rounded">
              <div className="text-2xl font-bold text-blue-600 dark:text-blue-400">{stats.total_entries}</div>
              <div className="text-xs text-gray-500 dark:text-gray-400">Total Entries</div>
            </div>
            <div className="text-center p-3 bg-gray-50 dark:bg-gray-700 rounded">
              <div className="text-2xl font-bold text-yellow-600 dark:text-yellow-400">{stats.directories}</div>
              <div className="text-xs text-gray-500 dark:text-gray-400">Directories</div>
            </div>
            <div className="text-center p-3 bg-gray-50 dark:bg-gray-700 rounded">
              <div className="text-2xl font-bold text-green-600 dark:text-green-400">{stats.files}</div>
              <div className="text-xs text-gray-500 dark:text-gray-400">Files</div>
            </div>
            <div className="text-center p-3 bg-gray-50 dark:bg-gray-700 rounded">
              <div className="text-2xl font-bold text-purple-600 dark:text-purple-400">{stats.symlinks}</div>
              <div className="text-xs text-gray-500 dark:text-gray-400">Symlinks</div>
            </div>
            <div className="text-center p-3 bg-gray-50 dark:bg-gray-700 rounded">
              <div className="text-2xl font-bold text-orange-600 dark:text-orange-400">{stats.local_overrides}</div>
              <div className="text-xs text-gray-500 dark:text-gray-400">Overrides</div>
            </div>
            <div className="text-center p-3 bg-gray-50 dark:bg-gray-700 rounded">
              <div className="text-2xl font-bold text-red-600 dark:text-red-400">{formatSize(stats.total_size_bytes)}</div>
              <div className="text-xs text-gray-500 dark:text-gray-400">Total Size</div>
            </div>
          </div>
        </div>
      )}

      {/* Recent Events */}
      <div className="bg-white dark:bg-gray-800 p-6 rounded-lg shadow">
        <h2 className="text-lg font-semibold text-gray-900 dark:text-white mb-4">Recent Events</h2>
        {events.length === 0 ? (
          <p className="text-gray-500 dark:text-gray-400 text-sm">No events yet. Events will appear here in real-time.</p>
        ) : (
          <div className="space-y-2 max-h-64 overflow-y-auto">
            {[...events].reverse().map((event, i) => (
              <div key={i} className="text-sm p-2 bg-gray-50 dark:bg-gray-700 rounded font-mono">
                <span className="text-gray-500 dark:text-gray-400">[{event.type}]</span>{' '}
                <span className="text-gray-900 dark:text-white">{JSON.stringify(event)}</span>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
