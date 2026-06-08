import React, { useEffect, useMemo, useRef, useState } from 'react';
import {
  Button,
  Empty,
  Input,
  InputNumber,
  Message,
  Modal,
  Select,
  Space,
  Spin,
  Table,
  Tabs,
  Tag,
  Typography,
} from '@arco-design/web-react';
import {
  IconCode,
  IconFile,
  IconFolder,
  IconPlayArrow,
  IconRefresh,
  IconStop,
} from '@arco-design/web-react/icon';
import { remoteApi } from '@/api/remote';
import type { RemoteAgent, RemoteCommand } from '@/api/remote';
import './RemoteAgents.css';

const TabPane = Tabs.TabPane;
const Option = Select.Option;

const formatTime = (time?: string) => (time ? new Date(time).toLocaleString('zh-CN') : '-');

const statusColor = (status: string) => {
  if (status === 'online') return 'green';
  if (status === 'disabled') return 'red';
  return 'gray';
};

const parseCommand = (payload: string) => {
  try {
    const data = JSON.parse(payload);
    return data.command || data.script_name || '-';
  } catch {
    return '-';
  }
};

const getServerOrigin = () => (typeof window === 'undefined' ? 'http://127.0.0.1:3000' : window.location.origin);

const RemoteAgents: React.FC = () => {
  const [agents, setAgents] = useState<RemoteAgent[]>([]);
  const [commands, setCommands] = useState<RemoteCommand[]>([]);
  const [selectedAgentId, setSelectedAgentId] = useState<number | null>(null);
  const [loadingAgents, setLoadingAgents] = useState(false);
  const [isCompact, setIsCompact] = useState(
    typeof window === 'undefined' ? false : window.innerWidth < 900
  );
  const [activeView, setActiveView] = useState('execute');
  const [command, setCommand] = useState('');
  const [scriptPath, setScriptPath] = useState('');
  const [scriptCommand, setScriptCommand] = useState('');
  const [workingDir, setWorkingDir] = useState('');
  const [timeout, setTimeoutValue] = useState<number>(0);
  const [currentCommandId, setCurrentCommandId] = useState<string | null>(null);
  const [logContent, setLogContent] = useState('');
  const [agentStatus, setAgentStatus] = useState<any | null>(null);
  const [filePath, setFilePath] = useState('.');
  const [fileEntries, setFileEntries] = useState<any[]>([]);
  const [fileContent, setFileContent] = useState('');
  const [selectedFilePath, setSelectedFilePath] = useState('');
  const [fileDirty, setFileDirty] = useState(false);
  const [fileLoading, setFileLoading] = useState(false);
  const [createModalVisible, setCreateModalVisible] = useState(false);
  const [newAgentName, setNewAgentName] = useState('');
  const [creatingAgent, setCreatingAgent] = useState(false);
  const [createdCommand, setCreatedCommand] = useState('');
  const pageRef = useRef<HTMLElement | null>(null);
  const eventSourceRef = useRef<EventSource | null>(null);

  const selectedAgent = useMemo(
    () => agents.find((agent) => agent.id === selectedAgentId) || null,
    [agents, selectedAgentId]
  );

  useEffect(() => {
    loadAgents();
    loadCommands();

    const updateCompact = (width?: number) => {
      const measuredWidth = width ?? pageRef.current?.getBoundingClientRect().width ?? window.innerWidth;
      setIsCompact(measuredWidth < 860);
    };
    updateCompact();

    const observer = typeof ResizeObserver !== 'undefined' ? new ResizeObserver((entries) => {
      updateCompact(entries[0]?.contentRect.width);
    }) : null;

    if (pageRef.current && observer) {
      observer.observe(pageRef.current);
    }

    const handleResize = () => updateCompact();
    window.addEventListener('resize', handleResize);

    return () => {
      window.removeEventListener('resize', handleResize);
      observer?.disconnect();
      eventSourceRef.current?.close();
    };
  }, []);

  useEffect(() => {
    if (!selectedAgentId) return;
    loadCommands(selectedAgentId);
    setFileEntries([]);
    setFileContent('');
    setSelectedFilePath('');
    setFileDirty(false);
  }, [selectedAgentId]);

  useEffect(() => {
    if (!selectedAgentId || activeView !== 'monitor') return;
    loadStatus();
    if (selectedAgent?.status !== 'online') return;
    const timer = window.setInterval(loadStatus, 10000);
    return () => window.clearInterval(timer);
  }, [activeView, selectedAgentId, selectedAgent?.status]);

  const loadAgents = async () => {
    setLoadingAgents(true);
    try {
      const data = await remoteApi.listAgents();
      setAgents(data);
      if (data.length === 0) {
        setSelectedAgentId(null);
        setActiveView('setup');
      } else if (!selectedAgentId) {
        setSelectedAgentId(data[0].id);
      }
    } catch (error: any) {
      Message.error(error.response?.data?.error || '加载远程机器失败');
    } finally {
      setLoadingAgents(false);
    }
  };

  const loadCommands = async (agentId?: number) => {
    try {
      setCommands(await remoteApi.listCommands(agentId));
    } catch {
      setCommands([]);
    }
  };

  const connectLogStream = (commandId: string) => {
    eventSourceRef.current?.close();
    const token = localStorage.getItem('token');
    const url = `/api/remote/commands/${commandId}/logs/stream${token ? `?token=${token}` : ''}`;
    const eventSource = new EventSource(url);
    eventSourceRef.current = eventSource;
    setActiveView('logs');

    eventSource.onmessage = (event) => {
      setLogContent((prev) => `${prev}${event.data}\n`);
    };
    eventSource.onerror = () => {
      eventSource.close();
      loadCommands(selectedAgentId || undefined);
    };
  };

  const runCommand = async () => {
    if (!selectedAgentId) {
      Message.warning('请选择远程机器');
      return;
    }
    if (!command.trim()) {
      Message.warning('请输入执行指令');
      return;
    }

    try {
      const created = await remoteApi.createCommand(selectedAgentId, {
        command: command.trim(),
        working_dir: workingDir.trim() || undefined,
        timeout: timeout || undefined,
      });
      setCurrentCommandId(created.id);
      setLogContent('');
      connectLogStream(created.id);
      Message.success('远程指令已下发');
      loadCommands(selectedAgentId);
    } catch (error: any) {
      Message.error(error.response?.data?.error || '下发远程指令失败');
    }
  };

  const runScript = async () => {
    if (!selectedAgentId) {
      Message.warning('请选择远程机器');
      return;
    }
    if (!scriptPath.trim()) {
      Message.warning('请输入脚本路径');
      return;
    }

    try {
      const created = await remoteApi.runScript(selectedAgentId, {
        path: scriptPath.trim(),
        command: scriptCommand.trim() || undefined,
        working_dir: workingDir.trim() || undefined,
        timeout: timeout || undefined,
      });
      setCurrentCommandId(created.id);
      setLogContent('');
      connectLogStream(created.id);
      Message.success('远程脚本已下发');
      loadCommands(selectedAgentId);
    } catch (error: any) {
      Message.error(error.response?.data?.error || '下发远程脚本失败');
    }
  };

  const stopCommand = async () => {
    if (!currentCommandId) return;
    try {
      await remoteApi.killCommand(currentCommandId);
      Message.success('已发送终止请求');
    } catch {
      Message.error('终止失败');
    }
  };

  const loadStatus = async () => {
    if (!selectedAgentId) {
      Message.warning('请选择远程机器');
      return;
    }
    try {
      const status = await remoteApi.getStatus(selectedAgentId);
      setAgentStatus(status);
    } catch {
      Message.error('获取远程状态失败');
    }
  };

  const listFiles = async (path = filePath) => {
    if (!selectedAgentId) {
      Message.warning('请选择远程机器');
      return;
    }
    setFileLoading(true);
    try {
      const result = await remoteApi.listFiles(selectedAgentId, path || '.');
      if (result.error) {
        Message.error(result.error);
        setFileEntries([]);
      } else {
        setFilePath(path || '.');
        setFileEntries(result.entries || []);
        setFileContent('');
        setSelectedFilePath('');
        setFileDirty(false);
      }
    } catch {
      Message.error('读取远程目录失败');
    } finally {
      setFileLoading(false);
    }
  };

  const readFile = async (path: string) => {
    if (!selectedAgentId) return;
    setFileLoading(true);
    try {
      const result = await remoteApi.readFile(selectedAgentId, path);
      if (result.error) {
        Message.error(result.error);
        setFileContent('');
      } else {
        setFileContent(result.content || '');
        setSelectedFilePath(path);
        setFileDirty(false);
      }
    } catch {
      Message.error('读取远程文件失败');
    } finally {
      setFileLoading(false);
    }
  };

  const copyText = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      Message.success('已复制');
    } catch {
      Message.warning('复制失败，请手动复制');
    }
  };

  const buildAgentCommand = (agentId: number, token: string) =>
    `.\\zhuque-agent.exe start --server ${getServerOrigin()} --agent-id ${agentId} --token ${token} --config .\\zhuque-agent.json --allowed-roots .`;

  const createAgent = async () => {
    if (!newAgentName.trim()) {
      Message.warning('请输入机器名称');
      return;
    }
    setCreatingAgent(true);
    try {
      const created = await remoteApi.createAgent({ name: newAgentName.trim() });
      const commandText = buildAgentCommand(created.agent.id, created.token);
      setCreatedCommand(commandText);
      setSelectedAgentId(created.agent.id);
      setActiveView('setup');
      await loadAgents();
      setCreateModalVisible(false);
      setNewAgentName('');
      Message.success('机器已创建');
    } catch (error: any) {
      Message.error(error.response?.data?.error || '创建机器失败');
    } finally {
      setCreatingAgent(false);
    }
  };

  const parentPath = (path: string) => {
    const normalized = path.replace(/\\/g, '/');
    const index = normalized.lastIndexOf('/');
    return index > 0 ? path.slice(0, index) : '.';
  };

  const joinPath = (base: string, name: string) => {
    const separator = base.includes('\\') ? '\\' : '/';
    if (!base || base === '.') return name;
    return `${base.replace(/[\\/]+$/, '')}${separator}${name}`;
  };

  const handleFileActionResult = async (result: any, successMessage: string, refresh = true) => {
    if (result.error) {
      Message.error(result.error);
      return false;
    }
    Message.success(successMessage);
    if (refresh) {
      await listFiles(filePath);
    }
    return true;
  };

  const saveFile = async () => {
    if (!selectedAgentId || !selectedFilePath) return;
    setFileLoading(true);
    try {
      const result = await remoteApi.writeFile(selectedAgentId, {
        path: selectedFilePath,
        content: fileContent,
      });
      if (await handleFileActionResult(result, '文件已保存', false)) {
        setFileDirty(false);
      }
    } catch {
      Message.error('保存远程文件失败');
    } finally {
      setFileLoading(false);
    }
  };

  const createDirectory = async () => {
    if (!selectedAgentId) return;
    const name = window.prompt('目录名称');
    if (!name?.trim()) return;
    setFileLoading(true);
    try {
      const result = await remoteApi.createDir(selectedAgentId, joinPath(filePath, name.trim()));
      await handleFileActionResult(result, '目录已创建');
    } catch {
      Message.error('创建目录失败');
    } finally {
      setFileLoading(false);
    }
  };

  const renameSelectedFile = async () => {
    if (!selectedAgentId || !selectedFilePath) return;
    const name = window.prompt('新名称', selectedFilePath.split(/[\\/]/).pop() || '');
    if (!name?.trim()) return;
    setFileLoading(true);
    try {
      const result = await remoteApi.renameFile(selectedAgentId, selectedFilePath, joinPath(parentPath(selectedFilePath), name.trim()));
      if (await handleFileActionResult(result, '已重命名')) {
        setSelectedFilePath('');
        setFileContent('');
        setFileDirty(false);
      }
    } catch {
      Message.error('重命名失败');
    } finally {
      setFileLoading(false);
    }
  };

  const deleteSelectedFile = async () => {
    if (!selectedAgentId || !selectedFilePath) return;
    Modal.confirm({
      title: '删除远程文件',
      content: `确定删除 ${selectedFilePath}？`,
      onOk: async () => {
        setFileLoading(true);
        try {
          const result = await remoteApi.deleteFile(selectedAgentId, selectedFilePath);
          if (await handleFileActionResult(result, '已删除')) {
            setSelectedFilePath('');
            setFileContent('');
            setFileDirty(false);
          }
        } catch {
          Message.error('删除失败');
        } finally {
          setFileLoading(false);
        }
      },
    });
  };

  const downloadFile = () => {
    if (!selectedFilePath) return;
    const blob = new Blob([fileContent], { type: 'text/plain;charset=utf-8' });
    const url = URL.createObjectURL(blob);
    const link = document.createElement('a');
    link.href = url;
    link.download = selectedFilePath.split(/[\\/]/).pop() || 'remote-file.txt';
    link.click();
    URL.revokeObjectURL(url);
  };

  const formatBytes = (value?: number) => {
    if (!value && value !== 0) return '-';
    const units = ['B', 'KB', 'MB', 'GB', 'TB'];
    let size = value;
    let index = 0;
    while (size >= 1024 && index < units.length - 1) {
      size /= 1024;
      index += 1;
    }
    return `${size.toFixed(index === 0 ? 0 : 1)} ${units[index]}`;
  };

  const formatDuration = (seconds?: number) => {
    if (!seconds && seconds !== 0) return '-';
    const days = Math.floor(seconds / 86400);
    const hours = Math.floor((seconds % 86400) / 3600);
    const minutes = Math.floor((seconds % 3600) / 60);
    if (days > 0) return `${days} 天 ${hours} 小时`;
    if (hours > 0) return `${hours} 小时 ${minutes} 分钟`;
    return `${minutes} 分钟`;
  };

  const commandColumns = [
    {
      title: '任务',
      dataIndex: 'payload',
      render: (payload: string, record: RemoteCommand) => (
        <Space direction="vertical" size={0}>
          <Typography.Text className="remote-command-text">{parseCommand(payload)}</Typography.Text>
          <Typography.Text type="secondary" className="remote-muted">
            {record.kind} / {formatTime(record.created_at)}
          </Typography.Text>
        </Space>
      ),
    },
    {
      title: '状态',
      dataIndex: 'status',
      width: 92,
      render: (status: string) => <Tag>{status}</Tag>,
    },
    {
      title: '',
      width: 72,
      render: (_: unknown, record: RemoteCommand) => (
        <Button
          size="mini"
          onClick={() => {
            setCurrentCommandId(record.id);
            setLogContent('');
            connectLogStream(record.id);
          }}
        >
          日志
        </Button>
      ),
    },
  ];

  const renderAgentCard = (agent: RemoteAgent) => (
    <button
      key={agent.id}
      className={`remote-agent-card ${agent.id === selectedAgentId ? 'is-selected' : ''}`}
      onClick={() => setSelectedAgentId(agent.id)}
    >
      <span className="remote-agent-main">
        <span className="remote-agent-name">{agent.name}</span>
        <span className="remote-agent-host">{agent.hostname || '-'}</span>
      </span>
      <span className="remote-agent-meta">
        <Tag color={statusColor(agent.status)}>{agent.status}</Tag>
        <span>{agent.os || '-'} / {agent.arch || '-'}</span>
      </span>
    </button>
  );

  const renderAgentSelector = () => {
    if (isCompact) {
      return (
        <section className="remote-mobile-agent-picker">
          <div className="remote-mobile-agent-row">
            <label className="remote-label" htmlFor="remote-agent-select">远程机器</label>
            <Space size={6}>
              <Button onClick={() => setCreateModalVisible(true)}>创建</Button>
              <Button icon={<IconRefresh />} onClick={loadAgents} aria-label="刷新远程机器" />
            </Space>
          </div>
          {agents.length > 0 ? (
            <Select
              id="remote-agent-select"
              value={selectedAgentId ?? undefined}
              onChange={(value) => setSelectedAgentId(value)}
              placeholder="选择远程机器"
            >
              {agents.map((agent) => (
                <Option key={agent.id} value={agent.id}>
                  {agent.name} / {agent.status}
                </Option>
              ))}
            </Select>
          ) : (
            <div className="remote-empty-inline">
              <Empty description="暂无远程机器" />
            </div>
          )}
        </section>
      );
    }

    return (
      <aside className="remote-sidebar">
        <div className="remote-sidebar-header">
          <Typography.Text bold>远程机器</Typography.Text>
          <Space size={6}>
            <Button onClick={() => setCreateModalVisible(true)}>创建</Button>
            <Button icon={<IconRefresh />} onClick={loadAgents} aria-label="刷新远程机器" />
          </Space>
        </div>
        <Spin loading={loadingAgents}>
          <div className="remote-agent-list remote-agent-list-sidebar">
            {agents.length > 0 ? agents.map(renderAgentCard) : (
              <div className="remote-sidebar-empty">
                <Empty description="暂无远程机器" />
              </div>
            )}
          </div>
        </Spin>
      </aside>
    );
  };

  const renderMobileAgentList = () => (
    <div className="remote-agent-list">
      <Button icon={<IconRefresh />} onClick={loadAgents} long>
        刷新机器
      </Button>
      {agents.length > 0 ? agents.map(renderAgentCard) : renderSetupGuide(true)}
    </div>
  );

  const renderCommandLine = (text: string) => (
    <div className="remote-command-line">
      <code>{text}</code>
      <Button size="mini" onClick={() => copyText(text)}>
        复制
      </Button>
    </div>
  );

  const renderSetupGuide = (compact = false) => {
    return (
      <section className={`remote-setup ${compact ? 'is-compact' : ''}`}>
        <div className="remote-setup-head">
          <Typography.Title heading={6} className="remote-setup-title">接入远程机器</Typography.Title>
          <Typography.Text type="secondary">
            远程机器需要先注册 agent，再保持 agent 在线。注册成功后这里会自动出现机器。
          </Typography.Text>
        </div>

        <div className="remote-setup-steps">
          <div className="remote-setup-step">
            <span className="remote-step-index">1</span>
            <div>
              <div className="remote-step-title">创建远程机器</div>
              <Typography.Text type="secondary">
                在本页面点击“创建”，服务端会创建机器并分配专属连接 token。
              </Typography.Text>
            </div>
          </div>

          <div className="remote-setup-step">
            <span className="remote-step-index">2</span>
            <div>
              <div className="remote-step-title">分发 agent 到远程机器</div>
              <Typography.Text type="secondary">
                使用与服务端同版本的 `zhuque-agent` 二进制，不建议在生产机器临时拉源码构建。
              </Typography.Text>
              {renderCommandLine('scp zhuque-agent.exe worker-1:C:\\zhuque\\zhuque-agent.exe')}
            </div>
          </div>

          <div className="remote-setup-step">
            <span className="remote-step-index">3</span>
            <div>
              <div className="remote-step-title">执行一键接入命令</div>
              <Typography.Text type="secondary">
                复制创建后生成的一条命令，在远程机器执行即可上线。`--allowed-roots` 控制文件页可访问目录，多个目录用英文分号分隔。
              </Typography.Text>
              {createdCommand ? renderCommandLine(createdCommand) : (
                <Button type="primary" onClick={() => setCreateModalVisible(true)}>
                  创建机器
                </Button>
              )}
            </div>
          </div>
        </div>
      </section>
    );
  };

  const renderExecute = () => (
    <div className="remote-workflow">
      {!selectedAgent ? renderSetupGuide() : (
        <>
      <div className="remote-workflow-grid">
        <section className="remote-fieldset">
          <div className="remote-fieldset-title">
            <IconCode aria-hidden />
            <span>执行指令</span>
          </div>
          <label className="remote-label" htmlFor="remote-command">指令内容</label>
          <Input.TextArea
            id="remote-command"
            value={command}
            onChange={setCommand}
            placeholder="例如: pwd 或 python script.py"
            autoSize={{ minRows: 5, maxRows: 8 }}
            spellCheck={false}
            className="remote-code-input"
          />
          <Button type="primary" icon={<IconPlayArrow />} onClick={runCommand} long={isCompact}>
            执行指令
          </Button>
        </section>

        <section className="remote-fieldset">
          <div className="remote-fieldset-title">
            <IconFile aria-hidden />
            <span>执行脚本</span>
          </div>
          <label className="remote-label" htmlFor="remote-script-path">脚本路径</label>
          <Input
            id="remote-script-path"
            value={scriptPath}
            onChange={setScriptPath}
            placeholder="例如: demo.py"
            spellCheck={false}
          />
          <label className="remote-label" htmlFor="remote-script-command">执行命令</label>
          <Input
            id="remote-script-command"
            value={scriptCommand}
            onChange={setScriptCommand}
            placeholder="可选，支持 {script}"
            spellCheck={false}
          />
          <Button icon={<IconPlayArrow />} onClick={runScript} long={isCompact}>
            执行脚本
          </Button>
        </section>
      </div>

      <section className="remote-run-options">
        <label className="remote-label" htmlFor="remote-working-dir">工作目录</label>
        <Input
          id="remote-working-dir"
          value={workingDir}
          onChange={setWorkingDir}
          placeholder="可选"
          spellCheck={false}
        />
        <label className="remote-label" htmlFor="remote-timeout">超时</label>
        <InputNumber
          id="remote-timeout"
          value={timeout}
          onChange={(value) => setTimeoutValue(Number(value || 0))}
          min={0}
          precision={0}
          suffix="秒"
        />
      </section>
        </>
      )}
    </div>
  );

  const renderMonitor = () => (
    <div className="remote-monitor">
      {!selectedAgent ? renderSetupGuide() : (
        <>
          <div className="remote-toolbar">
            <Button icon={<IconRefresh />} onClick={loadStatus}>
              刷新状态
            </Button>
          </div>
          {agentStatus ? (
            <div className="remote-status-grid" aria-live="polite">
              <section className="remote-status-card">
                <span className="remote-status-label">CPU</span>
                <strong>{typeof agentStatus.cpu_usage === 'number' ? `${agentStatus.cpu_usage.toFixed(1)}%` : '-'}</strong>
              </section>
              <section className="remote-status-card">
                <span className="remote-status-label">内存可用</span>
                <strong>{formatBytes(agentStatus.memory_available)}</strong>
                <span>{formatBytes(agentStatus.memory_total)} 总量</span>
              </section>
              <section className="remote-status-card">
                <span className="remote-status-label">运行时长</span>
                <strong>{formatDuration(agentStatus.uptime_seconds)}</strong>
              </section>
              <section className="remote-status-card">
                <span className="remote-status-label">最近在线</span>
                <strong>{formatTime(selectedAgent.last_seen_at)}</strong>
              </section>
              <section className="remote-status-detail">
                <div>
                  <span>主机</span>
                  <strong>{selectedAgent.hostname || '-'}</strong>
                </div>
                <div>
                  <span>系统</span>
                  <strong>{selectedAgent.os || '-'} / {selectedAgent.arch || '-'}</strong>
                </div>
                <div>
                  <span>Agent 版本</span>
                  <strong>{selectedAgent.version || '-'}</strong>
                </div>
              </section>
            </div>
          ) : (
            <div className="remote-empty-panel remote-status-empty">
              <Empty description="暂无状态数据" />
              <Button icon={<IconRefresh />} onClick={loadStatus}>
                刷新状态
              </Button>
            </div>
          )}
        </>
      )}
    </div>
  );

  const renderLogs = () => (
    <div className="remote-logs">
      {!selectedAgent ? renderSetupGuide() : (
        <>
          <div className="remote-toolbar">
            <Button status="danger" icon={<IconStop />} onClick={stopCommand} disabled={!currentCommandId}>
              终止当前
            </Button>
          </div>
          <pre className="remote-log-block" aria-live="polite">{logContent || '暂无日志'}</pre>
          {isCompact ? (
            <div className="remote-command-list">
              {commands.length > 0 ? commands.map((item) => (
                <button
                  key={item.id}
                  className="remote-command-card"
                  onClick={() => {
                    setCurrentCommandId(item.id);
                    setLogContent('');
                    connectLogStream(item.id);
                  }}
                >
                  <span className="remote-command-card-main">{parseCommand(item.payload)}</span>
                  <span className="remote-command-card-meta">
                    <Tag>{item.status}</Tag>
                    <span>{formatTime(item.created_at)}</span>
                  </span>
                </button>
              )) : <div className="remote-empty-panel"><Empty description="暂无命令记录" /></div>}
            </div>
          ) : (
            commands.length > 0
              ? <Table rowKey="id" columns={commandColumns} data={commands} pagination={{ pageSize: 8 }} />
              : <div className="remote-empty-panel"><Empty description="暂无命令记录" /></div>
          )}
        </>
      )}
    </div>
  );

  const renderFiles = () => (
    <div className="remote-files">
      {!selectedAgent ? renderSetupGuide() : (
        <>
          <div className="remote-path-row">
            <label className="remote-label" htmlFor="remote-file-path">路径</label>
            <Input
              id="remote-file-path"
              value={filePath}
              onChange={setFilePath}
              spellCheck={false}
            />
            <Button icon={<IconRefresh />} onClick={() => listFiles(filePath)}>
              打开
            </Button>
            <Button onClick={createDirectory}>
              新建目录
            </Button>
          </div>
          <Spin loading={fileLoading}>
            <div className="remote-file-browser">
              <section className="remote-file-list-panel">
                {isCompact ? (
                  <div className="remote-file-list">
                    {fileEntries.length > 0 ? fileEntries.map((entry) => (
                      <button
                        key={entry.path}
                        className="remote-file-card"
                        onClick={() => (entry.is_directory ? listFiles(entry.path) : readFile(entry.path))}
                      >
                        <span className="remote-file-name">
                          {entry.is_directory ? <IconFolder aria-hidden /> : <IconFile aria-hidden />}
                          {entry.name}
                        </span>
                        <span className="remote-muted">{entry.is_directory ? '目录' : `${entry.size || 0} B`}</span>
                      </button>
                    )) : <div className="remote-empty-panel"><Empty description="打开目录后显示文件" /></div>}
                  </div>
                ) : (
                  fileEntries.length > 0 ? (
                    <Table
                      rowKey="path"
                      data={fileEntries}
                      pagination={{ pageSize: 8 }}
                      columns={[
                        {
                          title: '名称',
                          dataIndex: 'name',
                          render: (name: string, record: any) => (
                            <Button
                              type="text"
                              icon={record.is_directory ? <IconFolder /> : <IconFile />}
                              onClick={() => (record.is_directory ? listFiles(record.path) : readFile(record.path))}
                            >
                              {name}
                            </Button>
                          ),
                        },
                        {
                          title: '类型',
                          width: 90,
                          render: (_: unknown, record: any) => (record.is_directory ? '目录' : '文件'),
                        },
                        { title: '大小', dataIndex: 'size', width: 120 },
                      ]}
                    />
                  ) : (
                    <div className="remote-empty-panel"><Empty description="打开目录后显示文件" /></div>
                  )
                )}
              </section>
              <section className="remote-file-preview-panel">
                <div className="remote-file-actions">
                  <Typography.Text className="remote-file-current">
                    {selectedFilePath || '未选择文件'}
                    {fileDirty ? ' *' : ''}
                  </Typography.Text>
                  <Space wrap>
                    <Button disabled={!selectedFilePath || !fileDirty} onClick={saveFile}>保存</Button>
                    <Button disabled={!selectedFilePath} onClick={downloadFile}>下载</Button>
                    <Button disabled={!selectedFilePath} onClick={renameSelectedFile}>重命名</Button>
                    <Button status="danger" disabled={!selectedFilePath} onClick={deleteSelectedFile}>删除</Button>
                  </Space>
                </div>
                {selectedFilePath ? (
                  <Input.TextArea
                    value={fileContent}
                    onChange={(value) => {
                      setFileContent(value);
                      setFileDirty(true);
                    }}
                    spellCheck={false}
                    className="remote-file-content"
                  />
                ) : (
                  <div className="remote-empty-panel"><Empty description="选择文件后显示内容" /></div>
                )}
              </section>
            </div>
          </Spin>
        </>
      )}
    </div>
  );

  const selectedTitle = selectedAgent ? selectedAgent.name : '未选择机器';

  return (
    <main className={`remote-page ${isCompact ? 'is-compact' : ''}`} ref={pageRef}>
      <div className="remote-shell">
        {renderAgentSelector()}

        <section className="remote-workspace">
          <header className="remote-workspace-header">
            <div>
              <Typography.Title heading={5} className="remote-title">{selectedTitle}</Typography.Title>
              <div className="remote-subtitle">
                <Tag color={statusColor(selectedAgent?.status || 'offline')}>
                  {selectedAgent?.status || 'offline'}
                </Tag>
                <span>{selectedAgent?.hostname || '-'}</span>
                <span>{selectedAgent?.os || '-'} / {selectedAgent?.arch || '-'}</span>
              </div>
            </div>
            {!isCompact && (
              <Button status="danger" icon={<IconStop />} onClick={stopCommand} disabled={!currentCommandId}>
                终止当前
              </Button>
            )}
          </header>

          <Tabs activeTab={activeView} onChange={setActiveView} className="remote-workspace-tabs">
            {isCompact && <TabPane key="machines" title="机器">{renderMobileAgentList()}</TabPane>}
            <TabPane key="setup" title="接入">{renderSetupGuide()}</TabPane>
            <TabPane key="execute" title="执行">{renderExecute()}</TabPane>
            <TabPane key="logs" title="日志">{renderLogs()}</TabPane>
            <TabPane key="files" title="文件">{renderFiles()}</TabPane>
            <TabPane key="monitor" title="状态">{renderMonitor()}</TabPane>
          </Tabs>
        </section>
      </div>
      <Modal
        title="创建远程机器"
        visible={createModalVisible}
        confirmLoading={creatingAgent}
        onOk={createAgent}
        onCancel={() => setCreateModalVisible(false)}
        okText="创建并生成命令"
      >
        <label className="remote-label" htmlFor="remote-new-agent-name">机器名称</label>
        <Input
          id="remote-new-agent-name"
          value={newAgentName}
          onChange={setNewAgentName}
          placeholder="例如: worker-1"
        />
        {createdCommand ? (
          <div className="remote-created-command">
            {renderCommandLine(createdCommand)}
          </div>
        ) : null}
      </Modal>
    </main>
  );
};

export default RemoteAgents;
