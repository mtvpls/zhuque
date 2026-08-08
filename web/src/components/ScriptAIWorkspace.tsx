import React, { useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import {
  Alert,
  Button,
  Card,
  Dropdown,
  Menu,
  Input,
  Message,
  Modal,
  Space,
  Tag,
} from '@arco-design/web-react';
import {
  IconCheck,
  IconArrowLeft,
  IconCode,
  IconDelete,
  IconClose,
  IconFile,
  IconFolder,
  IconHistory,
  IconInfoCircle,
  IconLoading,
  IconLock,
  IconEdit,
  IconUnlock,
  IconPlayArrow,
  IconPlus,
  IconRefresh,
  IconRobot,
  IconSend,
  IconStop,
} from '@arco-design/web-react/icon';
import './ScriptAIWorkspace.css';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';

type PermissionMode = 'default' | 'changes' | 'all';
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
  role: 'user' | 'assistant' | 'tool';
  content: string;
  metadata?: string | null;
}

interface AiSession {
  id: string;
  title: string;
  active_job_id?: string | null;
  current_context_tokens?: number | null;
  updated_at: string;
}

const estimateContextTokens = (parts: Array<string | undefined>) => {
  const characters = parts.reduce((total, part) => total + (part?.length || 0), 0);
  const messageOverhead = parts.filter((part) => Boolean(part?.trim())).length * 4;
  return Math.ceil(characters / 4) + messageOverhead;
};

const formatTokenCount = (value: number) => value > 1000
  ? `${(value / 1000).toFixed(1)}k`
  : value.toLocaleString();

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
  | { kind: 'message'; message: ConversationMessage; segmentId?: string };

type FeedGroup =
  | { kind: 'events'; events: AgentEvent[] }
  | { kind: 'message'; message: ConversationMessage };

const feedItemsFromConversation = (messages: ConversationMessage[]): FeedItem[] => {
  const feed: FeedItem[] = [];
  for (const message of messages) {
    if (message.role === 'user') {
      const isRuntimeContext = message.metadata?.includes('"runtime_context":true')
        || (message.content.startsWith('运行权限:') && message.content.includes('当前请求:'));
      if (!isRuntimeContext) feed.push({ kind: 'message', message });
      continue;
    }
    if (message.role === 'assistant') {
      let toolCalls: Array<{ function?: { name?: string; arguments?: string } }> = [];
      try {
        const metadata = message.metadata ? JSON.parse(message.metadata) as { tool_calls?: Array<{ function?: { name?: string; arguments?: string } }> } : {};
        toolCalls = metadata.tool_calls || [];
      } catch { }
      for (const call of toolCalls) {
        feed.push({ kind: 'event', event: { id: feed.length + 1, label: `调用工具 · ${call.function?.name || '未知工具'}`, detail: call.function?.arguments || undefined } });
      }
      if (message.content.trim()) feed.push({ kind: 'message', message });
      continue;
    }
    let toolName = '工具';
    try {
      const metadata = message.metadata ? JSON.parse(message.metadata) as { name?: string } : {};
      toolName = metadata.name || toolName;
    } catch { }
    if (!message.content.includes('系统拦截')) {
      feed.push({ kind: 'event', event: { id: feed.length + 1, label: `工具完成 · ${toolName}`, detail: message.content.slice(0, 1600), tone: 'success' } });
    }
  }
  return feed;
};

interface DiffLine {
  type: 'remove' | 'add' | 'gap';
  text: string;
  oldLine?: number;
  newLine?: number;
}

const formatToolApproval = (tool: string, argumentsValue: Record<string, unknown> | null) => {
  if (!argumentsValue) return '';
  if (tool === 'run_command') return String(argumentsValue.command || '');
  if (tool === 'run_script') return String(argumentsValue.path || '');
  if (tool === 'write_file' || tool === 'edit_file' || tool === 'delete_file') {
    return String(argumentsValue.path || '');
  }
  return JSON.stringify(argumentsValue, null, 2);
};

const buildDiffLines = (oldContent: string, newContent: string): DiffLine[] => {
  const oldLines = oldContent.split(/\r\n|\n|\r/);
  const newLines = newContent.split(/\r\n|\n|\r/);
  const oldCount = oldLines.length;
  const newCount = newLines.length;
  const maxCells = 4000000;
  const operations: Array<{ type: 'equal' | 'remove' | 'add'; oldIndex?: number; newIndex?: number }> = [];

  if (oldCount * newCount <= maxCells) {
    const lcs = Array.from({ length: oldCount + 1 }, () => new Uint32Array(newCount + 1));
    for (let oldIndex = oldCount - 1; oldIndex >= 0; oldIndex -= 1) {
      for (let newIndex = newCount - 1; newIndex >= 0; newIndex -= 1) {
        lcs[oldIndex][newIndex] = oldLines[oldIndex] === newLines[newIndex]
          ? lcs[oldIndex + 1][newIndex + 1] + 1
          : Math.max(lcs[oldIndex + 1][newIndex], lcs[oldIndex][newIndex + 1]);
      }
    }
    let oldIndex = 0;
    let newIndex = 0;
    while (oldIndex < oldCount && newIndex < newCount) {
      if (oldLines[oldIndex] === newLines[newIndex]) {
        operations.push({ type: 'equal', oldIndex, newIndex });
        oldIndex += 1;
        newIndex += 1;
      } else if (lcs[oldIndex + 1][newIndex] >= lcs[oldIndex][newIndex + 1]) {
        operations.push({ type: 'remove', oldIndex });
        oldIndex += 1;
      } else {
        operations.push({ type: 'add', newIndex });
        newIndex += 1;
      }
    }
    while (oldIndex < oldCount) {
      operations.push({ type: 'remove', oldIndex });
      oldIndex += 1;
    }
    while (newIndex < newCount) {
      operations.push({ type: 'add', newIndex });
      newIndex += 1;
    }
  } else {
    const maxCount = Math.max(oldCount, newCount);
    for (let index = 0; index < maxCount; index += 1) {
      if (oldLines[index] === newLines[index]) operations.push({ type: 'equal', oldIndex: index, newIndex: index });
      else {
        if (oldLines[index] !== undefined) operations.push({ type: 'remove', oldIndex: index });
        if (newLines[index] !== undefined) operations.push({ type: 'add', newIndex: index });
      }
    }
  }

  const changedIndexes = operations
    .map((operation, index) => operation.type === 'equal' ? -1 : index)
    .filter((index) => index >= 0);
  if (changedIndexes.length === 0) return [];

  const result: DiffLine[] = [];
  let blockStart = 0;
  for (let changedIndex = 0; changedIndex <= changedIndexes.length; changedIndex += 1) {
    const current = changedIndexes[changedIndex];
    const previous = changedIndexes[changedIndex - 1];
    if (changedIndex === changedIndexes.length || (current - previous > 1)) {
      const blockEnd = changedIndex === changedIndexes.length ? previous : previous;
      const before = blockStart === 0 ? 0 : changedIndexes[blockStart] - changedIndexes[blockStart - 1] - 1;
      if (blockStart > 0 && before > 0) result.push({ type: 'gap', text: '…' });
      for (let operationIndex = changedIndexes[blockStart]; operationIndex <= blockEnd; operationIndex += 1) {
        const operation = operations[operationIndex];
        if (operation.type === 'remove') {
          result.push({ type: 'remove', text: oldLines[operation.oldIndex!], oldLine: operation.oldIndex! + 1 });
        } else if (operation.type === 'add') {
          result.push({ type: 'add', text: newLines[operation.newIndex!], newLine: operation.newIndex! + 1 });
        }
      }
      blockStart = changedIndex;
    }
  }
  return result;
};

const renderDiffLine = (line: DiffLine, index: number, lines: DiffLine[]) => {
  if (line.type === 'gap') {
    return <><span className="diff-line-number">·</span><span className="diff-line-number">·</span><span>…</span></>;
  }

  const prefix = line.type === 'remove' ? '- ' : '+ ';
  const oldNumber = line.oldLine ? String(line.oldLine) : '';
  const newNumber = line.newLine ? String(line.newLine) : '';
  if (line.type !== 'add' || index === 0 || lines[index - 1].type !== 'remove') {
    return (
      <>
        <span className="diff-line-number">{oldNumber}</span>
        <span className="diff-line-number">{newNumber}</span>
        <span>{prefix}{line.text || ' '}</span>
      </>
    );
  }

  const previousText = lines[index - 1].text;
  const currentText = line.text;
  let start = 0;
  while (start < previousText.length && start < currentText.length && previousText[start] === currentText[start]) start += 1;
  let end = 0;
  while (
    end < previousText.length - start
    && end < currentText.length - start
    && previousText[previousText.length - 1 - end] === currentText[currentText.length - 1 - end]
  ) end += 1;

  const changedEnd = currentText.length - end;
  return (
    <>
      <span className="diff-line-number">{oldNumber}</span>
      <span className="diff-line-number">{newNumber}</span>
      <span>
        {prefix}{currentText.slice(0, start)}
        <span className="diff-char-add">{currentText.slice(start, changedEnd) || ' '}</span>
        {currentText.slice(changedEnd)}
      </span>
    </>
  );
};

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
  const [cacheUsage, setCacheUsage] = useState<{ hit: number; miss: number } | null>(null);
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [permissionMode, setPermissionMode] = useState<PermissionMode>('default');
  const [pendingPermission, setPendingPermission] = useState<'command' | 'change' | null>(null);
  const [pendingToolCallId, setPendingToolCallId] = useState<string | null>(null);
  const [pendingToolName, setPendingToolName] = useState('');
  const [pendingToolArguments, setPendingToolArguments] = useState<Record<string, unknown> | null>(null);
  const [showFullDiff, setShowFullDiff] = useState(false);
  const [showSessions, setShowSessions] = useState(false);
  const pendingApprovalRef = useRef<{ callId: string; decision: 'approve' | 'reject'; remember: boolean } | null>(null);
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
  const assistantSegmentRef = useRef<string | null>(null);
  const streamedTextRef = useRef(false);
  const toolInteractionRef = useRef(false);
  const conversationSyncRef = useRef(false);
  const reconnectTimerRef = useRef<number | null>(null);
  const keepSubscribedRef = useRef(false);
  const lastSequencesRef = useRef<Record<string, number>>({});
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
    if (!sessionId) {
      setProviderContextTokens(null);
      return;
    }
    const session = sessions.find((item) => item.id === sessionId);
    setProviderContextTokens(session?.current_context_tokens ?? null);
  }, [sessionId, sessions]);

  useEffect(() => {
    if (!sessionId || !visible) return;
    setLoadingSession(true);
    void fetch(`/api/ai/sessions/${encodeURIComponent(sessionId)}/messages`, {
      headers: { Authorization: `Bearer ${localStorage.getItem('token') || ''}` },
    }).then(async (response) => {
      if (!response.ok) return;
      const storedMessages = await response.json() as ConversationMessage[];
      const restoredFeed: FeedItem[] = [];
      for (const message of storedMessages) {
        if (message.role === 'user') {
          const isRuntimeContext = message.metadata?.includes('"runtime_context":true') || (message.content.startsWith('运行权限:') && message.content.includes('当前请求:'));
          if (!isRuntimeContext) restoredFeed.push({ kind: 'message', message });
          continue;
        }
        if (message.role === 'assistant') {
          let toolCalls: Array<{ function?: { name?: string; arguments?: string } }> = [];
          try {
            const metadata = message.metadata ? JSON.parse(message.metadata) as { tool_calls?: Array<{ function?: { name?: string; arguments?: string } }> } : {};
            toolCalls = metadata.tool_calls || [];
          } catch { }
          for (const call of toolCalls) {
            restoredFeed.push({ kind: 'event', event: { id: restoredFeed.length + 1, label: `调用工具 · ${call.function?.name || '未知工具'}`, detail: call.function?.arguments || undefined } });
          }
          if (message.content.trim()) restoredFeed.push({ kind: 'message', message });
          continue;
        }
        if (message.role === 'tool') {
          let toolName = '工具';
          try {
            const metadata = message.metadata ? JSON.parse(message.metadata) as { name?: string } : {};
            toolName = metadata.name || toolName;
          } catch { }
          if (!message.content.includes('系统拦截')) {
          restoredFeed.push({ kind: 'event', event: { id: restoredFeed.length + 1, label: `工具完成 · ${toolName}`, detail: message.content.slice(0, 1600), tone: 'success' } });
          }
        }
      }
      setConversation(storedMessages);
      setFeedItems(restoredFeed);
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
    contextEnabled ? fileContent : undefined,
    contextEnabled ? executionOutput : undefined,
  ]), [conversation, contextEnabled, fileContent, executionOutput]);

  const hasDiff = Boolean(draftChanges && draftChanges.length > 0);
  const currentChange = draftChanges?.find((change) => change.path === filePath);
  const pendingFileChange = useMemo<AgentFileChange | null>(() => {
    if (!pendingToolArguments || !['edit_file', 'write_file'].includes(pendingToolName)) return null;
    const path = String(pendingToolArguments.path || '');
    if (!path) return null;
    const backendDiff = pendingToolArguments.diff && typeof pendingToolArguments.diff === 'object'
      ? pendingToolArguments.diff as Record<string, unknown>
      : null;

    if (pendingToolName === 'write_file') {
      const content = String(backendDiff?.after ?? pendingToolArguments.content ?? '');
      const isCurrentFile = path === filePath || path === fileName;
      return { path, operation: isCurrentFile && fileContent ? 'update' : 'create', content };
    }

    if (typeof backendDiff?.after === 'string') {
      return { path, operation: 'update', content: backendDiff.after };
    }

    const oldString = String(pendingToolArguments.old_string || '');
    const newString = String(pendingToolArguments.new_string || '');
    const isCurrentFile = path === filePath || path === fileName;
    const source = isCurrentFile ? fileContent : oldString;
    const content = isCurrentFile
      ? (pendingToolArguments.replace_all === false
        ? source.replace(oldString, newString)
        : source.split(oldString).join(newString))
      : newString;
    return { path, operation: 'update', content };
  }, [fileContent, fileName, filePath, pendingToolArguments, pendingToolName]);

  const activePreviewChange = pendingFileChange || currentChange;
  const fullDiff = useMemo(() => {
    if (!activePreviewChange || activePreviewChange.content === undefined || activePreviewChange.operation === 'delete') return [];
    let oldContent = '';
    const backendDiff = pendingToolArguments?.diff && typeof pendingToolArguments.diff === 'object'
      ? pendingToolArguments.diff as Record<string, unknown>
      : null;
    if (typeof backendDiff?.before === 'string') oldContent = backendDiff.before;
    if (!oldContent && (activePreviewChange.path === filePath || activePreviewChange.path === fileName)) oldContent = fileContent;
    if (pendingToolName === 'edit_file' && !oldContent && pendingToolArguments) {
      oldContent = String(pendingToolArguments.old_string || '');
    }
    return buildDiffLines(oldContent, activePreviewChange.content);
  }, [activePreviewChange, fileContent, fileName, filePath, pendingToolArguments, pendingToolName]);
  const diffPreview = fullDiff.slice(0, 80);
  const diffTruncated = fullDiff.length > diffPreview.length;

  const addEvent = (label: string, detail?: string, tone: EventTone = 'neutral') => {
    setFeedItems((previous) => [
      ...previous,
      { kind: 'event', event: { id: Date.now() + previous.length, label, detail, tone } },
    ]);
  };


  const connectJob = (jobId?: string, startRequest?: Record<string, unknown>) => {
    const existingSocket = socketRef.current;
    if (startRequest && existingSocket?.readyState === WebSocket.OPEN) {
      existingSocket.send(JSON.stringify({ type: 'start', request: startRequest }));
      return existingSocket;
    }
    const token = localStorage.getItem('token') || '';
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const query = `token=${encodeURIComponent(token)}${jobId ? `&job_id=${encodeURIComponent(jobId)}` : ''}`;
    const socket = new WebSocket(`${protocol}//${window.location.host}/api/ai/ws?${query}`);
    socketRef.current = socket;
    keepSubscribedRef.current = true;

    socket.onopen = () => {
      if (startRequest) {
        socket.send(JSON.stringify({ type: 'start', request: startRequest }));
      }
      const queuedApproval = pendingApprovalRef.current;
      if (queuedApproval) {
        socket.send(JSON.stringify({
          type: queuedApproval.decision === 'approve' ? 'approve_tool' : 'reject_tool',
          tool_call_id: queuedApproval.callId,
          ...(queuedApproval.decision === 'approve' ? { remember: queuedApproval.remember } : {}),
        }));
        pendingApprovalRef.current = null;
        setPendingPermission(null);
        setPendingToolCallId(null);
        setPendingToolName('');
        setPendingToolArguments(null);
      }
      setRunning(Boolean(jobIdRef.current) || Boolean(startRequest));
    };

    socket.onmessage = (message) => {
      try {
        const envelope = JSON.parse(message.data) as { event?: Record<string, unknown>; seq?: number };
        const eventSessionId = typeof (envelope as Record<string, unknown>).session_id === 'string'
          ? String((envelope as Record<string, unknown>).session_id)
          : '';
        const eventJobId = typeof (envelope as Record<string, unknown>).job_id === 'string'
          ? String((envelope as Record<string, unknown>).job_id)
          : eventSessionId;
        const payload = (envelope.event || envelope) as Record<string, any>;
        const type = String(payload.type || '');
        const tool = typeof payload.tool === 'string' ? payload.tool : 'unknown';
        if (type === 'job_started' && eventSessionId && eventJobId) {
          sessionJobsRef.current[eventSessionId] = eventJobId;
          localStorage.setItem('script-ai-active-jobs', JSON.stringify(sessionJobsRef.current));
          setSessions((previous) => previous.map((session) => (
            session.id === eventSessionId ? { ...session, active_job_id: eventJobId } : session
          )));
          if (eventSessionId === sessionIdRef.current) jobIdRef.current = eventJobId;
        } else if (type === 'done' && eventSessionId && eventJobId) {
          if (sessionJobsRef.current[eventSessionId] === eventJobId) {
            delete sessionJobsRef.current[eventSessionId];
            localStorage.setItem('script-ai-active-jobs', JSON.stringify(sessionJobsRef.current));
          }
          setSessions((previous) => previous.map((session) => (
            session.id === eventSessionId && session.active_job_id === eventJobId
              ? { ...session, active_job_id: null }
              : session
          )));
          if (eventSessionId === sessionIdRef.current && jobIdRef.current === eventJobId) jobIdRef.current = null;
        }
        if (eventSessionId && eventSessionId !== sessionIdRef.current) return;
        const previousSequence = lastSequencesRef.current[eventJobId] || 0;
        if (typeof envelope.seq === 'number' && envelope.seq <= previousSequence) return;
        if (typeof envelope.seq === 'number') lastSequencesRef.current[eventJobId] = envelope.seq;
        if (type === 'context_usage') {
          const tokens = Number(payload.tokens);
          if (payload.source === 'provider' && Number.isFinite(tokens) && tokens > 0) {
            const roundedTokens = Math.round(tokens);
            setProviderContextTokens(roundedTokens);
            setSessions((previous) => previous.map((session) => (
              session.id === sessionIdRef.current
                ? { ...session, current_context_tokens: roundedTokens }
                : session
            )));
          }
        } else if (type === 'cache_usage') {
          const hit = Number(payload.hit_tokens);
          const miss = Number(payload.miss_tokens);
          if (Number.isFinite(hit) && Number.isFinite(miss)) {
            setCacheUsage({ hit: Math.max(0, hit), miss: Math.max(0, miss) });
          }
        } else if (type === 'context_compacting') {
          addEvent('正在压缩上下文', `当前约 ${formatTokenCount(Number(payload.tokens) || 0)} tokens`, 'warning');
        } else if (type === 'context_compacted') {
          const messages = Array.isArray(payload.messages)
            ? payload.messages.filter((item): item is ConversationMessage => Boolean(item && (item.role === 'user' || item.role === 'assistant' || item.role === 'tool') && typeof item.content === 'string'))
            : [];
          if (messages.length > 0) {
            conversationSyncRef.current = true;
            setConversation(messages);
            setFeedItems(feedItemsFromConversation(messages));
            setProviderContextTokens(null);
            setCacheUsage(null);
            setSessions((previous) => previous.map((session) => (
              session.id === sessionIdRef.current
                ? { ...session, current_context_tokens: null }
                : session
            )));
            assistantTextRef.current = '';
            assistantSegmentRef.current = null;
            streamedTextRef.current = false;
            toolInteractionRef.current = false;
          }
          addEvent('上下文已自动压缩', payload.source === 'local' ? 'Provider 压缩失败，已使用本地降级压缩' : '已清理并同步压缩后的会话消息', 'success');
        } else if (type === 'session_title') {
          const title = typeof payload.title === 'string' ? payload.title : '';
          if (title && sessionIdRef.current) {
            setSessions((previous) => previous.map((session) => (
              session.id === sessionIdRef.current ? { ...session, title } : session
            )));
          }
        } else if (type === 'job_started') {
          const id = eventJobId || null;
          if (id) jobIdRef.current = id;
        } else if (type === 'tool_call' || type === 'tool_result') {
          assistantSegmentRef.current = null;
          toolInteractionRef.current = true;
          const result = typeof payload.result === 'string' ? payload.result : '';
          if (type === 'tool_result' && payload.success === false && result.includes('系统拦截')) {
            setFeedItems((previous) => [
              ...previous,
              {
                kind: 'event',
                event: {
                  id: Date.now() + previous.length,
                  label: '工具被系统拦截 · ' + tool,
                  detail: result.slice(0, 1600) || '执行失败',
                  tone: 'error',
                },
              },
            ]);
            return;
          }
          if (type === 'tool_call') {
            setFeedItems((previous) => [
              ...previous,
              {
                kind: 'event',
                event: {
                  id: Date.now() + previous.length,
                  label: '调用工具 · ' + tool,
                  detail: payload.arguments ? JSON.stringify(payload.arguments) : undefined,
                  tone: 'neutral',
                },
              },
            ]);
            return;
          }
          setFeedItems((previous) => [
            ...previous,
            {
              kind: 'event',
              event: {
                id: Date.now() + previous.length,
                label: '工具完成 · ' + tool,
                detail: result.slice(0, 1600) || (payload.success ? '无输出' : '执行失败'),
                tone: payload.success ? 'success' : 'error',
              },
            },
          ]);
        } else if (type === 'approval_required') {
          const callId = typeof payload.tool_call_id === 'string' ? payload.tool_call_id : '';
          if (callId) {
            pendingApprovalRef.current = null;
            setPendingToolCallId(callId);
            setPendingToolName(tool);
            const approvalArguments = payload.arguments && typeof payload.arguments === 'object'
              ? { ...(payload.arguments as Record<string, unknown>), diff: payload.diff }
              : null;
            setPendingToolArguments(approvalArguments);
            setPendingPermission(payload.permission === 'change' ? 'change' : 'command');
          }
        } else if (type === 'text') {
          const chunk = typeof payload.content === 'string' ? payload.content : '';
          if (chunk) {
            assistantTextRef.current += chunk;
            streamedTextRef.current = true;
            if (!assistantSegmentRef.current) {
              assistantSegmentRef.current = `assistant-${Date.now()}-${envelope.seq ?? Math.random()}`;
            }
            const segmentId = assistantSegmentRef.current;
            setFeedItems((previous) => {
              const index = previous.findIndex((item) => item.kind === 'message' && item.segmentId === segmentId);
              if (index >= 0) {
                const item = previous[index];
                if (item.kind !== 'message') return previous;
                const next = [...previous];
                next[index] = { ...item, message: { ...item.message, content: item.message.content + chunk } };
                return next;
              }
              return [...previous, { kind: 'message', segmentId, message: { role: 'assistant', content: chunk } }];
            });
          }
        } else if (type === 'cancelled') {
          addEvent('任务已取消', String(payload.message || '当前任务已停止'), 'warning');
        } else if (type === 'conversation_sync') {
          const messages = Array.isArray(payload.messages)
            ? payload.messages.filter((item): item is ConversationMessage => Boolean(item && (item.role === 'user' || item.role === 'assistant' || item.role === 'tool') && typeof item.content === 'string'))
            : [];
          if (messages.length > 0) {
            conversationSyncRef.current = true;
            setConversation(messages);
          }
        } else if (type === 'error') {
          const errorMessage = String(payload.message || '无法连接 AI Agent');
          addEvent('Agent 请求失败', errorMessage, 'error');
          if (errorMessage.includes('后台任务不存在') && sessionIdRef.current) {
            delete sessionJobsRef.current[sessionIdRef.current];
            localStorage.setItem('script-ai-active-jobs', JSON.stringify(sessionJobsRef.current));
            keepSubscribedRef.current = false;
            if (sessionIdRef.current === sessionIdRef.current) {
              jobIdRef.current = null;
              activeRequestRef.current = '';
              setRunning(false);
            }
            socket.close();
          }
        } else if (type === 'done') {
          const finalAssistantMessage = assistantTextRef.current || 'Agent 已完成。';
          const request = activeRequestRef.current;
          if (request) {
            if (!streamedTextRef.current) {
              setFeedItems((previous) => [...previous, { kind: 'message', message: { role: 'assistant', content: finalAssistantMessage } }]);
            }
            if (!conversationSyncRef.current && !toolInteractionRef.current) {
              setConversation((previous) => [...previous, { role: 'user', content: request }, { role: 'assistant', content: finalAssistantMessage }]);
            }
          }
          activeRequestRef.current = '';
          if (sessionIdRef.current) {
            delete sessionJobsRef.current[sessionIdRef.current];
            localStorage.setItem('script-ai-active-jobs', JSON.stringify(sessionJobsRef.current));
          }
          if (sessionIdRef.current === sessionIdRef.current) jobIdRef.current = null;
          keepSubscribedRef.current = true;
          setRunning(false);
        }
      } catch (error: any) {
        addEvent('Agent 事件解析失败', error?.message || '无效事件', 'error');
      }
    };

    socket.onerror = () => {
      addEvent('AI 连接异常', jobIdRef.current ? '任务仍在服务端运行，连接恢复后会继续接收结果' : '会话连接将自动恢复', 'warning');
    };
    socket.onclose = () => {
      if (socketRef.current === socket) socketRef.current = null;
      if (keepSubscribedRef.current) {
        setRunning(Boolean(jobIdRef.current));
        reconnectTimerRef.current = window.setTimeout(() => {
          reconnectTimerRef.current = null;
          if (keepSubscribedRef.current && !socketRef.current) connectJob(jobIdRef.current || undefined);
        }, 1000);
      } else if (jobIdRef.current) {
        setRunning(false);
      }
    };
    return socket;
  };

  const handleSubmit = () => {
    const request = prompt.trim();
    if (!request || running) return;
    if (/^\/(compress|compact)$/i.test(request)) {
      setPrompt('');
      setCommandSuggestionsVisible(false);
      void compressSession();
      return;
    }
    pendingApprovalRef.current = null;
    setPendingPermission(null);
    setPendingToolCallId(null);
    setPendingToolName('');
    setPendingToolArguments(null);
    setRunning(true);
    assistantTextRef.current = '';
    assistantSegmentRef.current = null;
    streamedTextRef.current = false;
    toolInteractionRef.current = false;
    conversationSyncRef.current = false;
    setCacheUsage(null);
    setPrompt('');
    setDraftChanges(null);
    keepSubscribedRef.current = true;
    sessionIdRef.current = sessionId;
    lastSequencesRef.current = {};
    activeRequestRef.current = request;
    assistantTextRef.current = '';
    setFeedItems((previous) => [...previous, { kind: 'message', message: { role: 'user', content: request } }]);
    connectJob(jobIdRef.current || undefined, {
      mode: 'agent',
      prompt: request,
      file_name: contextEnabled ? fileName : undefined,
      file_path: contextEnabled ? filePath : undefined,
      file_content: contextEnabled ? fileContent : undefined,
      execution_output: contextEnabled ? executionOutput : undefined,
      directory_path: aiDirectoryPath,
      history: conversation,
      allow_commands: permissionMode === 'all',
      allow_changes: permissionMode === 'changes' || permissionMode === 'all',
      session_id: sessionId,
    });
  };


  const respondToToolApproval = (decision: 'approve' | 'reject', remember = false) => {
    if (!pendingToolCallId) return;
    const approval = { callId: pendingToolCallId, decision, remember };
    pendingApprovalRef.current = approval;
    const socket = socketRef.current;
    if (!socket || socket.readyState !== WebSocket.OPEN) {
      Message.info('审批决定已暂存，连接恢复后会自动提交');
      return;
    }
    socket.send(JSON.stringify({
      type: decision === 'approve' ? 'approve_tool' : 'reject_tool',
      tool_call_id: pendingToolCallId,
      ...(decision === 'approve' ? { remember } : {}),
    }));
    pendingApprovalRef.current = null;
    setPendingPermission(null);
    setPendingToolCallId(null);
    setPendingToolName('');
    setPendingToolArguments(null);
  };

  const stopAgent = () => {
    if (!running || !socketRef.current || socketRef.current.readyState !== WebSocket.OPEN) return;
    const jobId = jobIdRef.current || (sessionIdRef.current ? sessionJobsRef.current[sessionIdRef.current] : null);
    socketRef.current.send(JSON.stringify({ type: 'cancel', job_id: jobId || null }));
  };
  const compressSession = async () => {
    if (!sessionId || running || conversation.length < 2) return;
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
    if (visible && sessionId && jobIdRef.current && !socketRef.current) connectJob(jobIdRef.current);
  }, [visible, sessionId]);

  useEffect(() => {
    if (visible) return;
    keepSubscribedRef.current = false;
    if (reconnectTimerRef.current !== null) window.clearTimeout(reconnectTimerRef.current);
    reconnectTimerRef.current = null;
    socketRef.current?.close();
    socketRef.current = null;
  }, [visible]);

  useEffect(() => () => {
    keepSubscribedRef.current = false;
    if (reconnectTimerRef.current !== null) window.clearTimeout(reconnectTimerRef.current);
    reconnectTimerRef.current = null;
    socketRef.current?.close();
    socketRef.current = null;
  }, []);

  const applyDraft = async (remember = false) => {
    if (!draftChanges || draftChanges.length === 0) return;
    const changes = draftChanges;
    try {
      await onApplyChanges(changes);
      if (remember) setPermissionMode('changes');
      setPendingPermission(null);
      setPendingToolCallId(null);
      setPendingToolName('');
      setPendingToolArguments(null);
      setDraftChanges(null);
      addEvent('多文件修改已落盘', changes.length + ' 个文件立即生效', 'success');
      Message.success('修改已写入脚本工作区并立即生效');
    } catch (error: any) {
      addEvent('写入修改失败', error.message || '请检查文件权限', 'error');
      Message.error(error.message || '写入修改失败');
    }
  };

  useEffect(() => {
    if ((permissionMode === 'changes' || permissionMode === 'all') && draftChanges && draftChanges.length > 0 && !running) {
      void applyDraft();
    }
  }, [permissionMode, draftChanges, running]);


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
    sessionIdRef.current = value;
    jobIdRef.current = sessionJobsRef.current[value] || null;
    lastSequencesRef.current = {};
    activeRequestRef.current = '';
    keepSubscribedRef.current = true;
    setRunning(Boolean(jobIdRef.current));
    const socket = socketRef.current;
    if (socket?.readyState === WebSocket.OPEN && jobIdRef.current) {
      socket.send(JSON.stringify({ type: 'subscribe', job_id: jobIdRef.current }));
    }
    setSessionId(value);
    setDraftChanges(null);
    setPendingPermission(null);
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

  const workspace = (
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
                <div className="script-ai-message-content markdown-content">
                  {group.message.role === 'assistant' ? (
                    <ReactMarkdown remarkPlugins={[remarkGfm]}>{group.message.content}</ReactMarkdown>
                  ) : group.message.content}
                </div>
              </div>
            ))}
          </div>
        )}

        {pendingPermission && pendingToolCallId && (
          <div className="script-ai-permission">
            <strong>{pendingPermission === 'change' ? '该操作需要文件写入权限' : '该操作需要命令执行权限'}</strong>
            <pre className="script-ai-permission-command">{formatToolApproval(pendingToolName, pendingToolArguments)}</pre>
            {pendingFileChange && (
              <div className="script-ai-inline-diff">
                <div className="script-ai-diff-heading">
                  <span><IconCode /> 执行前变更预览 · {pendingFileChange.path}</span>
                  <Button
                    type="text"
                    size="mini"
                    onClick={() => setShowFullDiff(true)}
                    aria-label="查看完整文件 diff"
                    title="查看完整文件 diff"
                  >
                    查看全部
                  </Button>
                </div>
                {diffPreview.length > 0 ? (
                  <div className="script-ai-diff">
                    {diffPreview.map((line, index) => (
                      <div key={line.type + '-' + index} className={line.type === 'remove' ? 'diff-remove' : line.type === 'add' ? 'diff-add' : 'diff-gap'}>
                        {renderDiffLine(line, index, diffPreview)}
                      </div>
                    ))}
                  </div>
                ) : (
                  <pre className="script-ai-new-file-preview">{(pendingFileChange.content || '').slice(0, 2400)}</pre>
                )}
                {diffTruncated && <span className="script-ai-diff-more">已缩略显示 {fullDiff.length - diffPreview.length} 行，点击“查看全部”查看完整内容</span>}
              </div>
            )}
            <div className="script-ai-permission-actions">
              <Button size="small" type="primary" onClick={() => respondToToolApproval('approve')}>允许</Button>
              <Button size="small" onClick={() => respondToToolApproval('approve', true)}>本会话允许</Button>
              <Button size="small" onClick={() => respondToToolApproval('reject')}>拒绝</Button>
            </div>
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
                      <div key={line.type + '-' + index} className={line.type === 'remove' ? 'diff-remove' : line.type === 'add' ? 'diff-add' : 'diff-gap'}>
                        {renderDiffLine(line, index, diffPreview)}
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
              <Button size="small" icon={<IconRefresh />} onClick={() => { setDraftChanges(null); setPendingPermission(null); setPendingToolCallId(null); setPendingToolName(''); setPendingToolArguments(null); }}>拒绝</Button>
            </Space>
          </Card>
        )}
      </div>

      <Modal
        title={'完整变更 · ' + (activePreviewChange?.path || '')}
        visible={showFullDiff}
        onCancel={() => setShowFullDiff(false)}
        footer={null}
        wrapClassName="script-ai-full-diff-modal-wrap"
        className="script-ai-full-diff-modal"
      >
        {activePreviewChange && (
          <div className="script-ai-full-diff">
            {fullDiff.length > 0 ? fullDiff.map((line, index) => (
              <div key={line.type + '-' + index} className={line.type === 'remove' ? 'diff-remove' : line.type === 'add' ? 'diff-add' : 'diff-gap'}>
                {renderDiffLine(line, index, fullDiff)}
              </div>
            )) : (
              <pre>{activePreviewChange.content || ''}</pre>
            )}
          </div>
        )}
      </Modal>

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
              ? `${formatTokenCount(providerContextTokens)} tokens${cacheUsage && !running ? `（${formatTokenCount(cacheUsage.hit)}）` : ''}`
              : running
                ? '计算中...'
                : `估算约 ${formatTokenCount(currentContextTokens)} tokens`}
          </span>
          <Space>
            <Dropdown
              trigger="click"
              position="br"
              triggerProps={{ style: { zIndex: 2147483647 } }}
              droplist={(
                <Menu selectedKeys={[permissionMode]}>
                  <Menu.Item key="default" onClick={() => setPermissionMode('default')}><Space><IconLock />默认</Space></Menu.Item>
                  <Menu.Item key="changes" onClick={() => setPermissionMode('changes')}><Space><IconEdit />允许编辑文件</Space></Menu.Item>
                  <Menu.Item key="all" onClick={() => setPermissionMode('all')}><Space><IconUnlock />全部权限</Space></Menu.Item>
                </Menu>
              )}
            >
              <Button
                type="text"
                size="small"
                shape="circle"
                icon={permissionMode === 'default' ? <IconLock /> : permissionMode === 'changes' ? <IconEdit /> : <IconUnlock />}
                className="script-ai-permission-trigger"
                aria-label={`审批模式：${permissionMode === 'default' ? '默认' : permissionMode === 'changes' ? '允许编辑文件' : '全部权限'}`}
                title={`审批模式：${permissionMode === 'default' ? '默认' : permissionMode === 'changes' ? '允许编辑文件' : '全部权限'}`}
              />
            </Dropdown>
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
    </aside>);

  return isMobileViewport ? createPortal(workspace, document.body) : workspace;
};

export default ScriptAIWorkspace;
