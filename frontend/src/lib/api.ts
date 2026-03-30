import axios from 'axios';

const api = axios.create({ baseURL: '/api' });

let accessToken: string | null = null;
let refreshToken: string | null = null;

export function setTokens(access: string, refresh: string) {
  accessToken = access;
  refreshToken = refresh;
}

export function clearTokens() {
  accessToken = null;
  refreshToken = null;
}

export function getAccessToken() {
  return accessToken;
}

api.interceptors.request.use((config) => {
  if (accessToken) {
    config.headers.Authorization = `Bearer ${accessToken}`;
  }
  return config;
});

api.interceptors.response.use(
  (response) => response,
  async (error) => {
    if (error.response?.status === 401 && refreshToken) {
      try {
        const resp = await axios.post('/api/auth/refresh', { refresh_token: refreshToken });
        setTokens(resp.data.access_token, resp.data.refresh_token);
        error.config.headers.Authorization = `Bearer ${resp.data.access_token}`;
        return axios(error.config);
      } catch {
        clearTokens();
        window.location.href = '/login';
      }
    }
    return Promise.reject(error);
  }
);

export default api;
