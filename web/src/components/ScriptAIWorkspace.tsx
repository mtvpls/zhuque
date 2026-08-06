import React, { useEffect, useMemo, useRef, useState } from 'react';
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
  IconCode,
  IconClose,
  IconCopy,
  IconFile,
  IconFolder,
  IconInfoCircle,
  IconLoading,
  IconPlayArrow,
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
  const [feedItems, setFeedItems] = useState<FeedItem[]>([]);
  const [draftChanges, setDraftChanges] = useState<AgentFileChange[] | null>(null);
  const [running, setRunning] = useState(false);
  const [conversation, setConversation] = useState<ConversationMessage[]>([]);
  const [permissionMode, setPermissionMode] = useState<PermissionMode>('ask');
  const [pendingPermission, setPendingPermission] = useState<'command' | 'change' | null>(null);
  const [pendingRequest, setPendingRequest] = useState('');
  const [contextEnabled, setContextEnabled] = useState(Boolean(filePath || fileName));
  const [workspaceWidth, setWorkspaceWidth] = useState(360);
  const resizeState = useRef<{ startX: number; startWidth: number } | null>(null);
  const abortController = useRef<AbortController | null>(null);

  useEffect(() => {
    setContextEnabled(!aiDirectoryPath && Boolean(filePath || fileName));
  }, [aiDirectoryPath, filePath, fileName]);

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

  const handleSubmit = async (approvedCommand = false) => {
    const request = (approvedCommand ? pendingRequest : prompt).trim();
    if (!request || running) return;
    if (!approvedCommand && permissionMode !== 'session' && looksLikeCommandRequest(request)) {
      setPendingRequest(request);
      setPendingPermission('command');
      setFeedItems((previous) => [
        ...previous,
        { kind: 'message', message: { role: 'user', content: request } },
      ]);
      return;
    }

    setPendingPermission(null);
    setPendingRequest('');
    setRunning(true);
    setPrompt('');
    setDraftChanges(null);
    const controller = new AbortController();
    abortController.current = controller;
    if (!approvedCommand) {
      setFeedItems((previous) => [
        ...previous,
        { kind: 'message', message: { role: 'user', content: request } },
      ]);
    }
    addEvent('准备工作区上下文', aiDirectoryPath || filePath || fileName || '未选择文件');

    try {
      const token = localStorage.getItem('token');
      const response = await fetch('/api/ai/agent', {
        method: 'POST',
        headers: {
          Authorization: 'Bearer ' + token,
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          mode: 'agent',
          prompt: request,
          file_name: contextEnabled ? fileName : undefined,
          file_path: contextEnabled ? filePath : undefined,
          file_content: contextEnabled ? fileContent : undefined,
          execution_output: contextEnabled ? executionOutput : undefined,
          directory_path: aiDirectoryPath,
          history: conversation.slice(-12),
          allow_commands: approvedCommand || permissionMode === 'session',
        }),
        signal: controller.signal,
      });

      if (!response.ok) throw new Error(await response.text() || 'AI Agent 请求失败');
      if (!response.body) throw new Error('AI Agent 没有返回事件流');

      const reader = response.body.getReader();
      const decoder = new TextDecoder();
      let buffer = '';
      let streamedText = '';
      let assistantMessage = '';

      const consumeLine = (line: string) => {
        if (!line.startsWith('data:')) return;
        const raw = line.slice(5).trim();
        if (!raw) return;

        const payload = JSON.parse(raw) as {
          type?: string;
          tool?: string;
          success?: boolean;
          message?: string;
          result?: string;
          content?: string;
          summary?: string;
          files?: AgentFileChange[];
          arguments?: unknown;
        };

        if (payload.type === 'tool_call') {
          addEvent(
            '调用工具 · ' + (payload.tool || 'unknown'),
            payload.arguments ? JSON.stringify(payload.arguments) : undefined,
            'neutral',
          );
        } else if (payload.type === 'tool_result') {
          const resultDetail = payload.result
            ? payload.result.slice(0, 1600)
            : (payload.success ? '无输出' : '执行失败');
          addEvent(
            '工具完成 · ' + (payload.tool || 'unknown'),
            resultDetail,
            payload.success ? 'success' : 'error',
          );
        } else if (payload.type === 'text') {
          streamedText += payload.content || '';
          assistantMessage = streamedText;
        } else if (payload.type === 'changes') {
          const changes = (payload.files || []).filter((change) =>
            change &&
            typeof change.path === 'string' &&
            (change.operation === 'update' || change.operation === 'create' || change.operation === 'delete') &&
            (change.operation === 'delete' || typeof change.content === 'string'),
          );
          setDraftChanges(changes);
          assistantMessage = payload.summary || '已生成多文件修改提案，请检查 Diff。';
          addEvent('生成多文件修改提案', changes.length + ' 个文件等待应用', 'warning');
        } else if (payload.type === 'error') {
          throw new Error(payload.message || 'Agent 执行失败');
        }
      };

      while (true) {
        const result = await reader.read();
        if (result.done) break;
        buffer += decoder.decode(result.value, { stream: true });
        const lines = buffer.split('\n');
        buffer = lines.pop() || '';
        for (const line of lines) {
          try {
            consumeLine(line.trimEnd());
          } catch (error) {
            throw error instanceof Error ? error : new Error('Agent 事件解析失败');
          }
        }
      }
      if (buffer.trim()) consumeLine(buffer.trim());

      const finalAssistantMessage = assistantMessage || streamedText || 'Agent 已完成。';
      setConversation((previous) => [
        ...previous,
        { role: 'user', content: request },
        { role: 'assistant', content: finalAssistantMessage },
      ]);
      setFeedItems((previous) => [
        ...previous,
        { kind: 'message', message: { role: 'assistant', content: finalAssistantMessage } },
      ]);
    } catch (error: any) {
      setPendingPermission(null);
      if (error?.name === 'AbortError') {
        addEvent('已手动中断 AI 请求', '当前请求已停止', 'warning');
      } else {
        addEvent('Agent 请求失败', error.message || '无法连接 AI Agent', 'error');
      }
    } finally {
      if (abortController.current === controller) abortController.current = null;
      setRunning(false);
    }
  };

  const stopAgent = () => {
    if (!running || !abortController.current) return;
    abortController.current.abort();
  };

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

  if (!visible) return null;

  return (
    <aside
      className="script-ai-workspace"
      aria-label="AI 脚本工作台"
      style={{ width: workspaceWidth }}
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
            icon={<IconClose />}
            onClick={onClose}
            aria-label="关闭 AI 工作台"
            title="关闭 AI 工作台"
          />
        </Space>
      </div>

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

      <div className="script-ai-feed">
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
        <Input.TextArea
          value={prompt}
          onChange={setPrompt}
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
          <span><IconCopy /> Ctrl/Cmd + Enter 发送</span>
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
    </aside>
  );
};

export default ScriptAIWorkspace;
