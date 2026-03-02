import request from '@/utils/request';
import type { Dependence } from '@/types';

export const dependenceApi = {
  list: (type?: string) => request.get<Dependence[]>('/dependences', { params: { type } }),
  get: (id: number) => request.get<Dependence>(`/dependences/${id}`),
  create: (data: Partial<Dependence>) => request.post('/dependences', data),
  createBatch: (data: Array<{ name: string; type: string; remark?: string }>) => request.post('/dependences/batch', data),
  update: (id: number, data: Partial<Dependence>) => request.put(`/dependences/${id}`, data),
  delete: (id: number) => request.delete(`/dependences/${id}`),
  reinstall: (id: number) => request.post(`/dependences/${id}/reinstall`),
};
