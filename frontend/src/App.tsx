import { BrowserRouter, Routes, Route, Navigate } from 'react-router-dom';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { useEffect } from 'react';
import { initTheme } from './lib/theme';
import { getAccessToken } from './lib/api';
import Layout from './components/Layout';
import LoginPage from './pages/LoginPage';
import FilesPage from './pages/FilesPage';
import ConfigPage from './pages/ConfigPage';
import StatusPage from './pages/StatusPage';

const queryClient = new QueryClient();

function ProtectedRoute({ children }: { children: React.ReactNode }) {
  if (!getAccessToken()) {
    return <Navigate to="/login" replace />;
  }
  return <>{children}</>;
}

export default function App() {
  useEffect(() => { initTheme(); }, []);

  return (
    <QueryClientProvider client={queryClient}>
      <BrowserRouter>
        <Routes>
          <Route path="/login" element={<LoginPage />} />
          <Route path="/" element={<ProtectedRoute><Layout /></ProtectedRoute>}>
            <Route index element={<FilesPage />} />
            <Route path="config" element={<ConfigPage />} />
            <Route path="status" element={<StatusPage />} />
          </Route>
        </Routes>
      </BrowserRouter>
    </QueryClientProvider>
  );
}
