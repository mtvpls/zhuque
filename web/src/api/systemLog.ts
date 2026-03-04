import axios from 'axios';

export interface SystemLogEntry {
  timestamp: string;
  level: string;
  target: string;
  message: string;
}

export interface SystemLogsResponse {
  logs: SystemLogEntry[];
}

export const getSystemLogs = async (): Promise<SystemLogsResponse> => {
  const token = localStorage.getItem('token');
  const response = await axios.get<SystemLogsResponse>('/api/system/logs', {
    headers: { Authorization: `Bearer ${token}` },
  });
  return response.data;
};
