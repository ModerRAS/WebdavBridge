import { Outlet, NavLink, useNavigate } from 'react-router-dom';
import { clearTokens, getAccessToken } from '../lib/api';
import { getTheme, setTheme } from '../lib/theme';
import { useState } from 'react';
import { useWebSocket } from '../lib/useWebSocket';

export default function Layout() {
  const [theme, setThemeState] = useState(getTheme());
  const { connected } = useWebSocket();
  const navigate = useNavigate();

  const toggleTheme = () => {
    const newTheme = theme === 'dark' ? 'light' : 'dark';
    setTheme(newTheme);
    setThemeState(newTheme);
  };

  const handleLogout = () => {
    clearTokens();
    navigate('/login');
  };

  if (!getAccessToken()) {
    navigate('/login');
    return null;
  }

  const linkClass = ({ isActive }: { isActive: boolean }) =>
    `px-3 py-2 rounded-md text-sm font-medium ${isActive ? 'bg-gray-200 dark:bg-gray-700 text-gray-900 dark:text-white' : 'text-gray-600 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700'}`;

  return (
    <div className="min-h-screen bg-gray-50 dark:bg-gray-900">
      <nav className="bg-white dark:bg-gray-800 shadow">
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8">
          <div className="flex justify-between h-16">
            <div className="flex items-center space-x-4">
              <span className="text-xl font-bold text-gray-900 dark:text-white">WebdavBridge</span>
              <NavLink to="/" className={linkClass} end>Files</NavLink>
              <NavLink to="/config" className={linkClass}>Config</NavLink>
              <NavLink to="/status" className={linkClass}>Status</NavLink>
            </div>
            <div className="flex items-center space-x-3">
              <span className={`inline-block w-2 h-2 rounded-full ${connected ? 'bg-green-500' : 'bg-red-500'}`} title={connected ? 'Connected' : 'Disconnected'} />
              <button onClick={toggleTheme} className="p-2 rounded-md text-gray-500 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-700" title="Toggle theme">
                {theme === 'dark' ? '☀️' : '🌙'}
              </button>
              <button onClick={handleLogout} className="px-3 py-2 rounded-md text-sm font-medium text-gray-600 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700">
                Logout
              </button>
            </div>
          </div>
        </div>
      </nav>
      <main className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-6">
        <Outlet />
      </main>
    </div>
  );
}
