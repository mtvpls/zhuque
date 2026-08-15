import axios from 'axios';
import type { Subscription } from '@/types';

const API_BASE = '/api/subscriptions';

export interface CreateSubscription {
  name: string;
  url: string;
  subscription_type: 'git' | 'single_file';
  branch?: string;
  save_path?: string;
  auto_resolve_dependencies?: boolean;
  schedule: string;
  enabled?: boolean;
}

export interface UpdateSubscription {
  name?: string;
  url?: string;
  subscription_type?: 'git' | 'single_file';
  branch?: string;
  save_path?: string;
  auto_resolve_dependencies?: boolean;
  schedule?: string;
  enabled?: boolean;
}

export const subscriptionApi = {
  list: async (): Promise<Subscription[]> => {
    const token = localStorage.getItem('token');
    const res = await axios.get(API_BASE, {
      headers: { Authorization: `Bearer ${token}` },
    });
    return res.data;
  },

  get: async (id: number): Promise<Subscription> => {
    const token = localStorage.getItem('token');
    const res = await axios.get(`${API_BASE}/${id}`, {
      headers: { Authorization: `Bearer ${token}` },
    });
    return res.data;
  },

  create: async (data: CreateSubscription): Promise<Subscription> => {
    const token = localStorage.getItem('token');
    const res = await axios.post(API_BASE, data, {
      headers: { Authorization: `Bearer ${token}` },
    });
    return res.data;
  },

  update: async (id: number, data: UpdateSubscription): Promise<Subscription> => {
    const token = localStorage.getItem('token');
    const res = await axios.put(`${API_BASE}/${id}`, data, {
      headers: { Authorization: `Bearer ${token}` },
    });
    return res.data;
  },

  delete: async (id: number): Promise<void> => {
    const token = localStorage.getItem('token');
    await axios.delete(`${API_BASE}/${id}`, {
      headers: { Authorization: `Bearer ${token}` },
    });
  },

  run: async (id: number): Promise<{ log: string }> => {
    const token = localStorage.getItem('token');
    const res = await axios.post(`${API_BASE}/${id}/run`, {}, {
      headers: { Authorization: `Bearer ${token}` },
    });
    return res.data;
  },
};
