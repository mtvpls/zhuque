import axios from 'axios';
import type { AxiosInstance, AxiosRequestConfig } from 'axios';

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
      // 只有不在登录页面和初始设置页面时才清除token并重定向
      const currentPath = window.location.pathname;
      if (currentPath !== '/login' && currentPath !== '/setup') {
        localStorage.removeItem('token');
        window.location.href = '/login';
      }
    }
    return Promise.reject(error);
  }
);

export default request;
