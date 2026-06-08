import request from '@/utils/request';

export interface RemoteAgent {
  id: number;
  name: string;
  hostname?: string;
  os?: string;
  arch?: string;
  version?: string;
  status: string;
  last_seen_at?: string;
  registered_at: string;
  capabilities?: string;
  tags?: string;
  remark?: string;
  disabled: boolean;
}

export interface RemoteCommand {
  id: string;
  agent_id: number;
  kind: string;
  payload: string;
  status: string;
  exit_code?: number;
  output?: string;
  error?: string;
  timeout?: number;
  created_at: string;
  started_at?: string;
  finished_at?: string;
}

export interface RemoteCommandLog {
  id: number;
  command_id: string;
  stream: string;
  line: string;
  created_at: string;
}

export interface CreateRemoteAgentResponse {
  agent: RemoteAgent;
  token: string;
}

export const remoteApi = {
  listAgents: () => request.get<RemoteAgent[]>('/remote/agents'),
  createAgent: (payload: { name: string; remark?: string }) =>
    request.post<CreateRemoteAgentResponse>('/remote/agents', payload),
  getStatus: (agentId: number) => request.get<any>(`/remote/agents/${agentId}/status`),
  listFiles: (agentId: number, path: string) =>
    request.get<any>(`/remote/agents/${agentId}/files`, { params: { path } }),
  readFile: (agentId: number, path: string) =>
    request.get<any>(`/remote/agents/${agentId}/files/content`, { params: { path } }),
  writeFile: (agentId: number, payload: { path: string; content: string }) =>
    request.post<any>(`/remote/agents/${agentId}/files/content`, payload),
  deleteFile: (agentId: number, path: string) =>
    request.post<any>(`/remote/agents/${agentId}/files/delete`, { path }),
  createDir: (agentId: number, path: string) =>
    request.post<any>(`/remote/agents/${agentId}/files/mkdir`, { path }),
  renameFile: (agentId: number, from: string, to: string) =>
    request.post<any>(`/remote/agents/${agentId}/files/rename`, { from, to }),
  createCommand: (
    agentId: number,
    payload: { command: string; working_dir?: string; timeout?: number; env?: Record<string, string> }
  ) => request.post<RemoteCommand>(`/remote/agents/${agentId}/commands`, payload),
  runScript: (
    agentId: number,
    payload: { path: string; command?: string; working_dir?: string; timeout?: number; env?: Record<string, string> }
  ) => request.post<RemoteCommand>(`/remote/agents/${agentId}/scripts/run`, payload),
  listCommands: (agentId?: number) =>
    request.get<RemoteCommand[]>('/remote/commands', { params: agentId ? { agent_id: agentId } : undefined }),
  getCommand: (id: string) => request.get<RemoteCommand>(`/remote/commands/${id}`),
  killCommand: (id: string) => request.delete(`/remote/commands/${id}`),
  listCommandLogs: (id: string) => request.get<RemoteCommandLog[]>(`/remote/commands/${id}/logs`),
};
