import React, { useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import {
  Alert,
  Button,
  Card,
  Input,
  Message,
  Space,
  Tag,
} from '@arco-design/web-react';
import {
  IconCheck,
  IconArrowLeft,
  IconCode,
  IconDelete,
  IconClose,
  IconCopy,
  IconFile,
  IconFolder,
  IconHistory,
  IconInfoCircle,
  IconLoading,
  IconPlayArrow,
  IconPlus,
  IconRefresh,
  IconRobot,
  IconSend,
  IconStop,
} from '@arco-design/web-react/icon';
import './ScriptAIWorkspace.css';

type PermissionMode = 'ask' | 'session';
type EventTone = 'neutral' | 'success' | 'warning' | 'error';

interface AgentEvent {
  id: number;
  label: string;
  detail?: string;
  tone?: EventTone;
}

export interface AgentFileChange {
  path: string;
  operation: 'update' | 'create' | 'delete';
  content?: string;
}

interface ScriptAIWorkspaceProps {
  visible: boolean;
  fileName?: string;
  filePath?: string;
  fileContent: string;
  executionOutput?: string;
  aiDirectoryPath?: string;
  onClose: () => void;
  onRemoveDirectoryContext?: () => void;
  onApplyChanges: (changes: AgentFileChange[]) => Promise<void>;
}

interface ConversationMessage {
  role: 'user' | 'assistant';
  content: string;
}

interface AiSession {
  id: string;
  title: string;
  active_job_id?: string | null;
  updated_at: string;
}

const estimateContextTokens = (parts: Array<string | undefined>) => {
  const characters = parts.reduce((total, part) => total + (part?.length || 0), 0);
  const messageOverhead = parts.filter((part) => Boolean(part?.trim())).length * 4;
  return Math.ceil(characters / 4) + messageOverhead;
};

const formatSessionTime = (value: string) => {
  const timestamp = new Date(value.replace(' ', 'T') + 'Z');
  if (Number.isNaN(timestamp.getTime())) return value;
  return timestamp.toLocaleString('zh-CN', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  });
};

type FeedItem =
  | { kind: 'event'; event: AgentEvent }
  | { kind: 'message'; message: ConversationMessage };

type FeedGroup =
  | { kind: 'events'; events: AgentEvent[] }
  | { kind: 'message'; message: ConversationMessage };

const ScriptAIWorkspace: React.FC<ScriptAIWorkspaceProps> = ({
  visible,
  fileName,
  filePath,
  fileContent,
  executionOutput,
  aiDirectoryPath,
  onClose,
  onRemoveDirectoryContext,
  onApplyChanges,
}) => {
  const [prompt, setPrompt] = useState('');
  const [commandSuggestionsVisible, setCommandSuggestionsVisible] = useState(true);
  const [feedItems, setFeedItems] = useState<FeedItem[]>([]);
  const [draftChanges, setDraftChanges] = useState<AgentFileChange[] | null>(null);
  const [running, setRunning] = useState(false);
  const [conversation, setConversation] = useState<ConversationMessage[]>([]);
  const [sessions, setSessions] = useState<AiSession[]>([]);
  const [loadingSession, setLoadingSession] = useState(false);
  const [providerContextTokens, setProviderContextTokens] = useState<number | null>(null);
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [permissionMode, setPermissionMode] = useState<PermissionMode>('ask');
  const [pendingPermission, setPendingPermission] = useState<'command' | 'change' | null>(null);
  const [pendingRequest, setPendingRequest] = useState('');
  const [showSessions, setShowSessions] = useState(false);
  const [contextEnabled, setContextEnabled] = useState(Boolean(filePath || fileName));
  const [workspaceWidth, setWorkspaceWidth] = useState(360);
  const [isMobileViewport, setIsMobileViewport] = useState(() => window.matchMedia('(max-width: 1024px), (hover: none) and (pointer: coarse)').matches);
  const resizeState = useRef<{ startX: number; startWidth: number } | null>(null);
  const socketRef = useRef<WebSocket | null>(null);
  const storedJobs = JSON.parse(localStorage.getItem('script-ai-active-jobs') || '{}') as Record<string, string>;
  const sessionJobsRef = useRef<Record<string, string>>(storedJobs);
  const jobIdRef = useRef<string | null>(null);
  const activeRequestRef = useRef('');
  const assistantTextRef = useRef('');
  const reconnectTimerRef = useRef<number | null>(null);
  const keepSubscribedRef = useRef(false);
  const lastSequenceRef = useRef(0);
  const sessionIdRef = useRef<string | null>(null);
  const feedRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    setContextEnabled(!aiDirectoryPath && Boolean(filePath || fileName));
  }, [aiDirectoryPath, filePath, fileName]);

  useEffect(() => {
    const media = window.matchMedia('(max-width: 1024px), (hover: none) and (pointer: coarse)');
    const updateViewport = () => setIsMobileViewport(media.matches);
    updateViewport();
    media.addEventListener('change', updateViewport);
    return () => media.removeEventListener('change', updateViewport);
  }, []);

  useEffect(() => {
    if (!visible || !isMobileViewport) return;
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = 'hidden';
    return () => {
      document.body.style.overflow = previousOverflow;
    };
  }, [visible, isMobileViewport]);

  useEffect(() => {
    if (!visible) return;
    const headers = { Authorization: `Bearer ${localStorage.getItem('token') || ''}` };
    setLoadingSession(true);
    void (async () => {
      const response = await fetch('/api/ai/sessions', { headers });
      if (!response.ok) return;
      let list = await response.json() as AiSession[];
      if (list.length === 0) {
        const created = await fetch('/api/ai/sessions', { method: 'POST', headers });
        if (created.ok) list = [await created.json() as AiSession];
      }
      setSessions(list);
      for (const session of list) {
        if (session.active_job_id) sessionJobsRef.current[session.id] = session.active_job_id;
      }
      localStorage.setItem('script-ai-active-jobs', JSON.stringify(sessionJobsRef.current));
      setSessionId((current) => current && list.some((item) => item.id === current) ? current : list[0]?.id || null);
    })().catch(() => undefined).finally(() => setLoadingSession(false));
  }, [visible]);

  useEffect(() => {
    if (!sessionId || !visible) return;
    setLoadingSession(true);
    void fetch(`/api/ai/sessions/${encodeURIComponent(sessionId)}/messages`, {
      headers: { Authorization: `Bearer ${localStorage.getItem('token') || ''}` },
    }).then(async (response) => {
      if (!response.ok) return;
      const messages = await response.json() as ConversationMessage[];
      setConversation(messages);
      setFeedItems(messages.map((message) => ({ kind: 'message' as const, message })));
    }).catch(() => undefined).finally(() => setLoadingSession(false));
  }, [sessionId, visible]);

  const groupedFeed = useMemo<FeedGroup[]>(() => {
    const groups: FeedGroup[] = [];
    for (const item of feedItems) {
      if (item.kind === 'event') {
        const last = groups[groups.length - 1];
        if (last?.kind === 'events') {
          last.events.push(item.event);
        } else {
          groups.push({ kind: 'events', events: [item.event] });
        }
      } else {
        groups.push({ kind: 'message', message: item.message });
      }
    }
    return groups;
  }, [feedItems]);

  useEffect(() => {
    const frame = window.requestAnimationFrame(() => {
      const feed = feedRef.current;
      if (feed) feed.scrollTop = feed.scrollHeight;
    });
    return () => window.cancelAnimationFrame(frame);
  }, [feedItems, running, draftChanges]);

  const currentContextTokens = useMemo(() => estimateContextTokens([
    ...conversation.map((message) => message.content),
    prompt,
    contextEnabled ? fileContent : undefined,
    contextEnabled ? executionOutput : undefined,
  ]), [conversation, prompt, contextEnabled, fileContent, executionOutput]);

  const hasDiff = Boolean(draftChanges && draftChanges.length > 0);
  const currentChange = draftChanges?.find((change) => change.path === filePath);
  const diffPreview = useMemo(() => {
    if (!currentChange || currentChange.content === undefined || currentChange.operation === 'delete') {
      return [];
    }
    const oldLines = fileContent.split('\n');
    const newLines = currentChange.content.split('\n');
    const preview: Array<{ type: 'remove' | 'add'; text: string }> = [];
    const maxLines = Math.max(oldLines.length, newLines.length);
    for (let index = 0; index < maxLines; index += 1) {
      if (oldLines[index] !== newLines[index] && oldLines[index] !== undefined) {
        preview.push({ type: 'remove', text: oldLines[index] });
      }
      if (oldLines[index] !== newLines[index] && newLines[index] !== undefined) {
        preview.push({ type: 'add', text: newLines[index] });
      }
    }
    return preview.slice(0, 120);
  }, [currentChange, fileContent]);

  const addEvent = (label: string, detail?: string, tone: EventTone = 'neutral') => {
    setFeedItems((previous) => [
      ...previous,
      { kind: 'event', event: { id: Date.now() + previous.length, label, detail, tone } },
    ]);
  };

  const looksLikeCommandRequest = (request: string) =>
    /(执行|运行|命令|测试|编译|构建|安装依赖|启动|调试|shell|终端|lint|test|build|install|run|compile|ping|curl|wget|npm|pnpm|yarn|cargo|python|node|bash|powershell|cmd)/i.test(request);

  const getPendingCommand = (request: string) => {
    const quoted = request.match(/`([^`]+)`/);
    if (quoted?.[1]) return quoted[1].trim();
    return request
      .replace(/^\s*(请|帮我|麻烦)?\s*(执行|运行|调用|使用)?\s*(命令|shell|终端)?\s*[:：]?\s*/i, '')
      .trim();
  };

  const connectJob = (jobId?: string, startRequest?: Record<string, unknown>, socketSessionId = sessionIdRef.current) => {
    const token = localStorage.getItem('token') || '';
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const query = `token=${encodeURIComponent(token)}${jobId ? `&job_id=${encodeURIComponent(jobId)}` : ''}`;
    const socket = new WebSocket(`${protocol}//${window.location.host}/api/ai/ws?${query}`);
    socketRef.current = socket;
    assistantTextRef.current = '';
    keepSubscribedRef.current = true;

    socket.onopen = () => {
      if (startRequest) {
        socket.send(JSON.stringify({ type: 'start', request: startRequest }));
      }
      setRunning(true);
    };

    socket.onmessage = (message) => {
      try {
        const envelope = JSON.parse(message.data) as { event?: Record<string, unknown>; seq?: number };
        if (typeof envelope.seq === 'number' && envelope.seq <= lastSequenceRef.current) return;
        if (typeof envelope.seq === 'number') lastSequenceRef.current = envelope.seq;
        if (socketSessionId !== sessionIdRef.current) return;
        const payload = (envelope.event || envelope) as Record<string, any>;
        const type = String(payload.type || '');
        const tool = typeof payload.tool === 'string' ? payload.tool : 'unknown';
        if (type === 'context_usage') {
          const tokens = Number(payload.tokens);
          if (payload.source === 'provider' && Number.isFinite(tokens) && tokens > 0) {
            setProviderContextTokens(Math.round(tokens));
          }
        } else if (type === 'session_title') {
          const title = typeof payload.title === 'string' ? payload.title : '';
          if (title && socketSessionId) {
            setSessions((previous) => previous.map((session) => (
              session.id === socketSessionId ? { ...session, title } : session
            )));
          }
        } else if (type === 'job_started') {
          const id = typeof payload.job_id === 'string' ? payload.job_id : null;
          if (id) {
            jobIdRef.current = id;
            if (socketSessionId) {
              sessionJobsRef.current[socketSessionId] = id;
              localStorage.setItem('script-ai-active-jobs', JSON.stringify(sessionJobsRef.current));
            }
          }
        } else if (type === 'tool_call') {
          addEvent('调用工具 · ' + tool, payload.arguments ? JSON.stringify(payload.arguments) : undefined);
        } else if (type === 'tool_result') {
          const result = typeof payload.result === 'string' ? payload.result : '';
          addEvent('工具完成 · ' + tool, result.slice(0, 1600) || (payload.success ? '无输出' : '执行失败'), payload.success ? 'success' : 'error');
        } else if (type === 'text') {
          assistantTextRef.current += typeof payload.content === 'string' ? payload.content : '';
        } else if (type === 'changes') {
          const changes = (Array.isArray(payload.files) ? payload.files : []).filter((change): change is AgentFileChange => {
            const item = change as AgentFileChange;
            return Boolean(item && typeof item.path === 'string' &&
              (item.operation === 'update' || item.operation === 'create' || item.operation === 'delete') &&
              (item.operation === 'delete' || typeof item.content === 'string'));
          });
          setDraftChanges(changes);
          assistantTextRef.current = String(payload.summary || '已生成多文件修改提案，请检查 Diff。');
          addEvent('生成多文件修改提案', changes.length + ' 个文件等待应用', 'warning');
        } else if (type === 'cancelled') {
          addEvent('任务已取消', String(payload.message || '当前任务已停止'), 'warning');
        } else if (type === 'error') {
          const errorMessage = String(payload.message || '无法连接 AI Agent');
          addEvent('Agent 请求失败', errorMessage, 'error');
          if (errorMessage.includes('后台任务不存在') && socketSessionId) {
            delete sessionJobsRef.current[socketSessionId];
            localStorage.setItem('script-ai-active-jobs', JSON.stringify(sessionJobsRef.current));
            if (socketSessionId === sessionIdRef.current) {
              jobIdRef.current = null;
              keepSubscribedRef.current = false;
            }
          }
        } else if (type === 'done') {
          const finalAssistantMessage = assistantTextRef.current || 'Agent 已完成。';
          const request = activeRequestRef.current;
          if (request) {
            setConversation((previous) => [...previous, { role: 'user', content: request }, { role: 'assistant', content: finalAssistantMessage }]);
            setFeedItems((previous) => [...previous, { kind: 'message', message: { role: 'assistant', content: finalAssistantMessage } }]);
          }
          activeRequestRef.current = '';
          if (socketSessionId) {
            delete sessionJobsRef.current[socketSessionId];
            localStorage.setItem('script-ai-active-jobs', JSON.stringify(sessionJobsRef.current));
          }
          if (socketSessionId === sessionIdRef.current) jobIdRef.current = null;
          keepSubscribedRef.current = false;
          setRunning(false);
          socket.close();
        }
      } catch (error: any) {
        addEvent('Agent 事件解析失败', error?.message || '无效事件', 'error');
      }
    };

    socket.onerror = () => {
      addEvent('AI 连接异常', '任务仍在服务端运行，重新打开工作台后会继续接收结果', 'warning');
    };
    socket.onclose = () => {
      if (socketRef.current === socket) socketRef.current = null;
      if (keepSubscribedRef.current && jobIdRef.current) {
        setRunning(true);
        reconnectTimerRef.current = window.setTimeout(() => {
          reconnectTimerRef.current = null;
          if (keepSubscribedRef.current && socketSessionId === sessionIdRef.current && jobIdRef.current && !socketRef.current) connectJob(jobIdRef.current, undefined, socketSessionId);
        }, 1000);
      } else if (jobIdRef.current) {
        setRunning(false);
      }
    };
    return socket;
  };

  const handleSubmit = (approvedCommand = false) => {
    const request = (approvedCommand ? pendingRequest : prompt).trim();
    if (!request || running) return;
    if (!approvedCommand && /^\/(compress|compact)$/i.test(request)) {
      setPrompt('');
      setCommandSuggestionsVisible(false);
      void compressSession();
      return;
    }
    if (!approvedCommand && permissionMode !== 'session' && looksLikeCommandRequest(request)) {
      setPendingRequest(request);
      setPendingPermission('command');
      setFeedItems((previous) => [...previous, { kind: 'message', message: { role: 'user', content: request } }]);
      return;
    }

    setPendingPermission(null);
    setPendingRequest('');
    setRunning(true);
    setProviderContextTokens(null);
    setPrompt('');
    setDraftChanges(null);
    keepSubscribedRef.current = true;
    sessionIdRef.current = sessionId;
    lastSequenceRef.current = 0;
    activeRequestRef.current = request;
    assistantTextRef.current = '';
    if (!approvedCommand) {
      setFeedItems((previous) => [...previous, { kind: 'message', message: { role: 'user', content: request } }]);
    }
    addEvent('准备工作区上下文', aiDirectoryPath || filePath || fileName || '未选择文件');
    connectJob(undefined, {
      mode: 'agent',
      prompt: request,
      file_name: contextEnabled ? fileName : undefined,
      file_path: contextEnabled ? filePath : undefined,
      file_content: contextEnabled ? fileContent : undefined,
      execution_output: contextEnabled ? executionOutput : undefined,
      directory_path: aiDirectoryPath,
      history: conversation,
      allow_commands: approvedCommand || permissionMode === 'session',
      session_id: sessionId,
    });
  };

  const stopAgent = () => {
    if (!running || !socketRef.current || socketRef.current.readyState !== WebSocket.OPEN) return;
    socketRef.current.send(JSON.stringify({ type: 'cancel' }));
  };
  const compressSession = async () => {
    if (!sessionId || running || conversation.length < 2) return;
    const confirmed = window.confirm('压缩当前会话的历史上下文？压缩后将用摘要替换旧消息，无法恢复原始历史。');
    if (!confirmed) return;
    setFeedItems((previous) => [...previous, {
      kind: 'event' as const,
      event: { id: Date.now(), label: '开始压缩上下文', detail: `正在整理当前 ${conversation.length} 条历史消息并生成摘要`, tone: 'warning' as const },
    }]);
    try {
      const response = await fetch(`/api/ai/sessions/${encodeURIComponent(sessionId)}/messages`, {
        method: 'POST',
        headers: {
          Authorization: `Bearer ${localStorage.getItem('token') || ''}`,
          Accept: 'application/json',
        },
      });
      const responseText = await response.text();
      let result: {
        messages?: ConversationMessage[];
        before_tokens?: number;
        after_tokens?: number;
        before_messages?: number;
        after_messages?: number;
        compressed?: boolean;
        token_source?: string;
        error?: string;
      };
      try {
        result = JSON.parse(responseText) as typeof result;
      } catch {
        const preview = responseText.replace(/\s+/g, ' ').trim().slice(0, 180);
        throw new Error(`压缩接口返回了非 JSON 响应（HTTP ${response.status}）：${preview || '响应为空'}`);
      }
      if (!response.ok || !Array.isArray(result.messages)) throw new Error(result.error || `压缩失败（HTTP ${response.status}）`);
      const beforeTokens = result.before_tokens || 0;
      const afterTokens = result.after_tokens || 0;
      const ratio = beforeTokens > 0 ? Math.round((1 - afterTokens / beforeTokens) * 100) : 0;
      const didCompress = result.compressed === true && afterTokens < beforeTokens;
      const beforeMessages = result.before_messages || 0;
      const afterMessages = result.after_messages || 0;
      const tokenSource = result.token_source === 'provider+estimate' ? 'Provider usage 校准' : '本地估算（Provider 未返回 usage）';
      setConversation(result.messages);
      setFeedItems((previous) => [
        ...previous.filter((item) => item.kind === 'event'),
        {
          kind: 'event' as const,
          event: {
            id: Date.now(),
            label: '上下文压缩结束',
            detail: didCompress
              ? `已将 ${beforeMessages} 条消息压缩为 ${afterMessages} 条，约 ${beforeTokens} → ${afterTokens} tokens（减少 ${Math.max(0, ratio)}%，${tokenSource}）`
              : `当前历史没有发生有效压缩：${beforeMessages} 条消息，约 ${beforeTokens} → ${afterTokens} tokens（${tokenSource}）`,
            tone: didCompress ? 'success' as const : 'warning' as const,
          },
        },
        ...result.messages!.map((message) => ({ kind: 'message' as const, message })),
      ]);
      if (didCompress) Message.success('当前会话已压缩');
      else Message.warning('当前历史没有发生有效压缩');
    } catch (error) {
      const detail = error instanceof Error ? error.message : '压缩失败';
      setFeedItems((previous) => [...previous, {
        kind: 'event' as const,
        event: { id: Date.now(), label: '上下文压缩结束', detail, tone: 'error' as const },
      }]);
      Message.error(detail);
    }
  };

  useEffect(() => {
    sessionIdRef.current = sessionId;
    jobIdRef.current = sessionId ? sessionJobsRef.current[sessionId] || null : null;
    keepSubscribedRef.current = visible;
    if (visible && sessionId && jobIdRef.current && !socketRef.current) connectJob(jobIdRef.current, undefined, sessionId);
    return () => {
      keepSubscribedRef.current = false;
      if (reconnectTimerRef.current !== null) window.clearTimeout(reconnectTimerRef.current);
      reconnectTimerRef.current = null;
      socketRef.current?.close();
      socketRef.current = null;
    };
  }, [visible, sessionId]);

  const applyDraft = async (remember = false) => {
    if (!draftChanges || draftChanges.length === 0) return;
    const changes = draftChanges;
    try {
      await onApplyChanges(changes);
      if (remember) setPermissionMode('session');
      setPendingPermission(null);
      setDraftChanges(null);
      addEvent('多文件修改已落盘', changes.length + ' 个文件立即生效', 'success');
      Message.success('修改已写入脚本工作区并立即生效');
    } catch (error: any) {
      addEvent('写入修改失败', error.message || '请检查文件权限', 'error');
      Message.error(error.message || '写入修改失败');
    }
  };

  useEffect(() => {
    if (permissionMode === 'session' && draftChanges && draftChanges.length > 0 && !running) {
      void applyDraft();
    }
  }, [permissionMode, draftChanges, running]);

  const copyPrompt = async () => {
    await navigator.clipboard?.writeText(prompt);
    Message.success('请求已复制');
  };

  useEffect(() => {
    const handlePointerMove = (event: PointerEvent) => {
      if (!resizeState.current) return;
      const maxWidth = Math.max(320, Math.min(720, window.innerWidth - 420));
      const nextWidth = resizeState.current.startWidth - (event.clientX - resizeState.current.startX);
      setWorkspaceWidth(Math.min(maxWidth, Math.max(320, nextWidth)));
    };
    const stopResize = () => {
      resizeState.current = null;
      document.body.style.cursor = '';
      document.body.style.userSelect = '';
    };
    window.addEventListener('pointermove', handlePointerMove);
    window.addEventListener('pointerup', stopResize);
    window.addEventListener('pointercancel', stopResize);
    return () => {
      window.removeEventListener('pointermove', handlePointerMove);
      window.removeEventListener('pointerup', stopResize);
      window.removeEventListener('pointercancel', stopResize);
    };
  }, []);

  const startResize = (event: React.PointerEvent<HTMLDivElement>) => {
    event.preventDefault();
    resizeState.current = { startX: event.clientX, startWidth: workspaceWidth };
    document.body.style.cursor = 'ew-resize';
    document.body.style.userSelect = 'none';
  };

  const selectSession = (value: string) => {
    keepSubscribedRef.current = false;
    socketRef.current?.close();
    socketRef.current = null;
    sessionIdRef.current = value;
    jobIdRef.current = sessionJobsRef.current[value] || null;
    lastSequenceRef.current = 0;
    activeRequestRef.current = '';
    setRunning(false);
    setSessionId(value);
    setDraftChanges(null);
    setPendingPermission(null);
    setPendingRequest('');
    setShowSessions(false);
  };

  const createSession = async () => {
    const response = await fetch('/api/ai/sessions', {
      method: 'POST',
      headers: { Authorization: 'Bearer ' + (localStorage.getItem('token') || '') },
    });
    if (!response.ok) return;
    const created = await response.json() as AiSession;
    setSessions((previous) => [created, ...previous]);
    selectSession(created.id);
    setConversation([]);
    setFeedItems([]);
    setDraftChanges(null);
  };

  const deleteSession = async (session: AiSession) => {
    if (!window.confirm('确定删除这个 AI 会话及其聊天记录吗？')) return;
    const response = await fetch('/api/ai/sessions/' + encodeURIComponent(session.id), {
      method: 'DELETE',
      headers: { Authorization: 'Bearer ' + (localStorage.getItem('token') || '') },
    });
    if (!response.ok) return;
    const remaining = sessions.filter((item) => item.id !== session.id);
    setSessions(remaining);
    delete sessionJobsRef.current[session.id];
    localStorage.setItem('script-ai-active-jobs', JSON.stringify(sessionJobsRef.current));
    if (session.id !== sessionId) return;
    socketRef.current?.close();
    socketRef.current = null;
    if (remaining.length > 0) {
      selectSession(remaining[0].id);
    } else {
      await createSession();
    }
  };

  if (!visible) return null;

  return createPortal(
    <aside
      className="script-ai-workspace"
      aria-label="AI 脚本工作台"
      style={isMobileViewport ? {
        position: 'fixed',
        zIndex: 2147483647,
        top: 0,
        right: 0,
        bottom: 0,
        left: 0,
        width: '100vw',
        maxWidth: '100vw',
        height: '100dvh',
        minHeight: '100dvh',
        margin: 0,
        border: 0,
        borderRadius: 0,
        boxShadow: 'none',
      } : { width: workspaceWidth }}
    >
      <div
        className="script-ai-resize-handle"
        onPointerDown={startResize}
        role="separator"
        aria-orientation="vertical"
        aria-valuemin={320}
        aria-valuemax={720}
        aria-valuenow={workspaceWidth}
        aria-label="调整 AI 工作台宽度"
      />
      <div className="script-ai-header">
        <div className="script-ai-title">
          <span className="script-ai-mark"><IconRobot /></span>
          <div>
            <strong>AI 脚本工作台</strong>
            <span>辅助修改脚本</span>
          </div>
        </div>
        <Space>
          <Button
            type="text"
            size="small"
            icon={<IconRefresh />}
            onClick={() => {
              setConversation([]);
              setFeedItems([]);
                        setDraftChanges(null);
            }}
            aria-label="清空对话"
            title="清空对话"
          />
          <Button
            type="text"
            size="small"
            icon={<IconHistory />}
            onClick={() => setShowSessions(true)}
            aria-label="切换会话"
            title="切换会话"
          />
          <Button
            type="text"
            size="small"
            icon={<IconClose />}
            onClick={onClose}
            aria-label="关闭 AI 工作台"
            title="关闭 AI 工作台"
          />
        </Space>
      </div>

      {showSessions ? (
        <div className="script-ai-sessions-page">
          <div className="script-ai-sessions-header">
            <Button
              type="text"
              size="small"
              icon={<IconArrowLeft />}
              onClick={() => setShowSessions(false)}
              aria-label="返回当前会话"
              title="返回当前会话"
            />
            <strong>AI 会话</strong>
            <Button
              type="text"
              size="small"
              icon={<IconPlus />}
              onClick={() => { void createSession(); }}
              aria-label="新建会话"
              title="新建会话"
            />
          </div>
          <div className="script-ai-session-list">
            {sessions.map((session) => (
              <div
                key={session.id}
                className={'script-ai-session-item' + (session.id === sessionId ? ' active' : '')}
                onClick={() => selectSession(session.id)}
                onKeyDown={(event) => {
                  if (event.key === 'Enter' || event.key === ' ') {
                    event.preventDefault();
                    selectSession(session.id);
                  }
                }}
                role="button"
                tabIndex={0}
              >
                <span className="script-ai-session-main">
                  <strong>{session.title || '未命名会话'}</strong>
                  <small>{formatSessionTime(session.updated_at)}</small>
                </span>
                <span className="script-ai-session-actions">
                  {session.active_job_id && <IconLoading spin />}
                  <Button
                    type="text"
                    size="small"
                    icon={<IconDelete />}
                    onClick={(event) => {
                      event.stopPropagation();
                      void deleteSession(session);
                    }}
                    aria-label={'删除会话 ' + (session.title || '未命名会话')}
                    title="删除会话"
                  />
                </span>
              </div>
            ))}
          </div>
        </div>
      ) : (
        <>
      <div className="script-ai-context">
        <div className="script-ai-context-row script-ai-context-main-row">
          <div className="script-ai-section-label">当前上下文</div>
          {contextEnabled ? (
            <>
              <Tag color="blue"><IconFile /> {fileName || '未选择文件'}</Tag>
              {filePath && filePath.replace(/[\\/]?[^\\/]+$/, '') && (
                <span className="script-ai-path">{filePath.replace(/[\\/]?[^\\/]+$/, '')}</span>
              )}
              <Button
                type="text"
                size="small"
                icon={<IconClose />}
                onClick={() => setContextEnabled(false)}
                aria-label="移除当前脚本上下文"
                title="移除当前脚本上下文"
              />
            </>
          ) : (
            !aiDirectoryPath && <span className="script-ai-context-empty">未附加脚本上下文</span>
          )}
          {aiDirectoryPath && (
            <>
              <Tag color="green"><IconFolder /> 目录</Tag>
              <span className="script-ai-path">{aiDirectoryPath}</span>
              <Button
                type="text"
                size="small"
                className="script-ai-context-remove"
                icon={<IconClose />}
                onClick={onRemoveDirectoryContext}
                aria-label="移除 AI 目录上下文"
                title="移除 AI 目录上下文"
              />
            </>
          )}
        </div>
        {contextEnabled && executionOutput && (
          <Tag color="orange" className="script-ai-output-tag"><IconPlayArrow /> 已附加最近一次执行输出</Tag>
        )}
      </div>

      <div className="script-ai-feed" ref={feedRef}>
        {loadingSession && (
          <div className="script-ai-session-loading"><IconLoading spin /> 正在加载会话...</div>
        )}
        <Alert
          type="info"
          icon={<IconInfoCircle />}
          content="描述你的脚本修改需求，AI 会协助分析并生成修改方案。"
          className="script-ai-notice"
        />
        {groupedFeed.length > 0 && (
          <div className="script-ai-feed-items">
            {groupedFeed.map((group, index) => group.kind === 'events' ? (
              <details key={'events-' + index} className="script-ai-event-group" open>
                <summary>调用过程 <span>{group.events.length} 条记录</span></summary>
                <div className="script-ai-event-group-body">
                  {group.events.map((event) => (
                    <div key={event.id} className={'script-ai-event tone-' + (event.tone || 'neutral')}>
                      <span className="script-ai-event-icon"><IconCheck /></span>
                      <span><strong>{event.label}</strong>{event.detail && <small>{event.detail}</small>}</span>
                    </div>
                  ))}
                </div>
              </details>
            ) : (
              <div key={'message-' + index} className={'script-ai-message role-' + group.message.role}>
                <span className="script-ai-message-role">{group.message.role === 'user' ? '你' : 'AI'}</span>
                <div className="script-ai-message-content">{group.message.content}</div>
              </div>
            ))}
          </div>
        )}

        {pendingPermission === 'command' && (
          <div className="script-ai-permission">
            <div><strong>将执行命令</strong><code>{getPendingCommand(pendingRequest)}</code></div>
            <Space>
              <Button size="small" type="primary" onClick={() => handleSubmit(true)}>允许</Button>
              <Button size="small" onClick={() => { setPermissionMode('session'); void handleSubmit(true); }}>本会话允许</Button>
              <Button size="small" onClick={() => { setPendingPermission(null); setPendingRequest(''); }}>拒绝</Button>
            </Space>
          </div>
        )}

        {running && (
          <div className="script-ai-thinking"><IconLoading spin /> AI 正在处理请求...</div>
        )}

        {hasDiff && draftChanges && (
          <Card className="script-ai-diff-card" bordered={false}>
            <div className="script-ai-diff-heading">
              <span><IconCode /> 待应用多文件修改</span>
              <Tag color="orange">{draftChanges.length} 个文件</Tag>
            </div>
            {draftChanges.map((change) => (
              <div className="script-ai-file-change" key={change.path}>
                <div className="script-ai-file-change-heading">
                  <span>{change.operation === 'delete' ? '删除' : change.operation === 'create' ? '新建' : '修改'} · {change.path}</span>
                </div>
                {change.path === filePath && diffPreview.length > 0 ? (
                  <div className="script-ai-diff">
                    {diffPreview.map((line, index) => (
                      <div key={line.type + '-' + index} className={line.type === 'remove' ? 'diff-remove' : 'diff-add'}>
                        {(line.type === 'remove' ? '- ' : '+ ') + (line.text || ' ')}
                      </div>
                    ))}
                  </div>
                ) : change.operation !== 'delete' ? (
                  <pre className="script-ai-new-file-preview">{(change.content || '').slice(0, 12000)}</pre>
                ) : (
                  <div className="script-ai-delete-preview">该文件将在应用后删除</div>
                )}
              </div>
            ))}
            <Space className="script-ai-diff-actions">
              <Button type="primary" size="small" icon={<IconCheck />} onClick={() => applyDraft(false)}>允许</Button>
              <Button size="small" onClick={() => applyDraft(true)}>本会话允许</Button>
              <Button size="small" icon={<IconRefresh />} onClick={() => { setDraftChanges(null); setPendingPermission(null); }}>拒绝</Button>
            </Space>
          </Card>
        )}
      </div>

      <div className="script-ai-composer">
        {commandSuggestionsVisible && prompt.trimStart().startsWith('/') && !prompt.includes(' ') && !running && (
          <div className="script-ai-command-suggestions" role="listbox" aria-label="可用指令">
            {['/compress'].filter((command) => command.startsWith(prompt.trim())).map((command) => (
              <button
                key={command}
                type="button"
                role="option"
                onMouseDown={(event) => event.preventDefault()}
                onClick={() => {
                  setPrompt(command);
                  setCommandSuggestionsVisible(false);
                }}
              >
                <code>{command}</code>
                <span>压缩当前会话上下文</span>
              </button>
            ))}
          </div>
        )}
        <Input.TextArea
          value={prompt}
          onChange={(value) => {
            setPrompt(value);
            setProviderContextTokens(null);
            setCommandSuggestionsVisible(value.trimStart().startsWith('/') && !value.includes(' '));
          }}
          autoSize={{ minRows: 3, maxRows: 6 }}
          placeholder="描述任务，例如：找到所有重试逻辑，统一改成指数退避并运行相关脚本。"
          onPressEnter={(event) => {
            if ((event.ctrlKey || event.metaKey) && !event.shiftKey) {
              event.preventDefault();
              handleSubmit();
            }
          }}
        />
        <div className="script-ai-composer-footer">
          <span>
            当前上下文：{providerContextTokens !== null
              ? `${providerContextTokens.toLocaleString()} tokens`
              : `约 ${currentContextTokens.toLocaleString()} tokens`}
          </span>
          <Space>
            <Button type="text" size="small" icon={<IconCopy />} onClick={copyPrompt} disabled={!prompt.trim()} aria-label="复制请求" />
            <Button
              type="text"
              size="small"
              icon={<IconStop />}
              onClick={stopAgent}
              disabled={!running}
              aria-label="中断 AI 请求"
              title="中断 AI 请求"
            />
            <Button type="primary" size="small" icon={<IconSend />} onClick={() => handleSubmit()} disabled={!prompt.trim() || running}>发送</Button>
          </Space>
        </div>
      </div>
        </>
      )}
    </aside>,
    document.body,
  );
};

export default ScriptAIWorkspace;
