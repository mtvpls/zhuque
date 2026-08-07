import axios from 'axios';
import type { AxiosInstance, AxiosRequestConfig } from 'axios';

let redirectingToLogin = false;

export const redirectToLoginOnUnauthorized = () => {
  const currentPath = window.location.pathname;

  if (currentPath === '/login' || currentPath === '/setup' || redirectingToLogin) {
    return;
  }

  redirectingToLogin = true;
  localStorage.removeItem('token');
  window.location.replace('/login');
};

export const installAuthErrorHandler = () => {
  if (typeof window === 'undefined') {
    return;
  }

  // Direct axios calls and fetch calls must use the same auth-expiration behavior.
  axios.interceptors.response.use(
    (response) => response,
    (error) => {
      if (error.response?.status === 401) {
        redirectToLoginOnUnauthorized();
      }
      return Promise.reject(error);
    },
  );

  const originalFetch = window.fetch.bind(window);
  window.fetch = async (...args: Parameters<typeof window.fetch>) => {
    const response = await originalFetch(...args);
    if (response.status === 401) {
      redirectToLoginOnUnauthorized();
    }
    return response;
  };
};

// 创建自定义的 axios 实例类型，返回数据而不是完整响应
interface CustomAxiosInstance extends Omit<AxiosInstance, 'get' | 'post' | 'put' | 'delete' | 'patch'> {
  get<T = any>(url: string, config?: AxiosRequestConfig): Promise<T>;
  post<T = any>(url: string, data?: any, config?: AxiosRequestConfig): Promise<T>;
  put<T = any>(url: string, data?: any, config?: AxiosRequestConfig): Promise<T>;
  delete<T = any>(url: string, config?: AxiosRequestConfig): Promise<T>;
  patch<T = any>(url: string, data?: any, config?: AxiosRequestConfig): Promise<T>;
}

const request = axios.create({
  baseURL: '/api',
  timeout: 30000,
}) as CustomAxiosInstance;

// 请求拦截器
request.interceptors.request.use(
  (config) => {
    const token = localStorage.getItem('token');
    if (token) {
      config.headers.Authorization = `Bearer ${token}`;
    }
    return config;
  },
  (error) => {
    return Promise.reject(error);
  }
);

// 响应拦截器
request.interceptors.response.use(
  (response) => {
    return response.data;
  },
  (error) => {
    if (error.response?.status === 401) {
      redirectToLoginOnUnauthorized();
    }
    return Promise.reject(error);
  }
);

export default request;
