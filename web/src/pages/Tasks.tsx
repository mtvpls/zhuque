import React, { useEffect, useState, useRef } from 'react';
import {
  Card,
  Table,
  Button,
  Space,
  Modal,
  Form,
  Input,
  Switch,
  Message,
  Popconfirm,
  Tag,
  Select,
  Tabs,
  Grid,
  Divider,
  Spin,
  Dropdown,
  Menu,
  Typography,
} from '@arco-design/web-react';
import { IconPlus, IconPlayArrow, IconEdit, IconDelete, IconInfoCircle, IconStop, IconFile, IconMore, IconLink } from '@arco-design/web-react/icon';
import { taskApi } from '@/api/task';
import { logApi } from '@/api/log';
import axios from 'axios';
import type { Task } from '@/types';

const FormItem = Form.Item;
const { Option } = Select;
const { Row, Col } = Grid;
const TabPane = Tabs.TabPane;

const Tasks: React.FC = () => {
  const [tasks, setTasks] = useState<Task[]>([]);
  const [runningTasks, setRunningTasks] = useState<Set<number>>(new Set());
  const [loading, setLoading] = useState(false);
  const [visible, setVisible] = useState(false);
  const [editingTask, setEditingTask] = useState<Task | null>(null);
  const [form] = Form.useForm();
  const [isMobile, setIsMobile] = useState(window.innerWidth < 768);

  // 分组相关状态
  const [groups, setGroups] = useState<any[]>([]);
  const [activeTab, setActiveTab] = useState<string>('all');
  const [groupManageVisible, setGroupManageVisible] = useState(false);
  const [groupModalVisible, setGroupModalVisible] = useState(false);
  const [editingGroup, setEditingGroup] = useState<any>(null);
  const [groupForm] = Form.useForm();

  // 日志相关状态
  const [logVisible, setLogVisible] = useState(false);
  const [logContent, setLogContent] = useState('');
  const [logLoading, setLogLoading] = useState(false);
  const [isLiveLog, setIsLiveLog] = useState(false);
  const [currentViewTask, setCurrentViewTask] = useState<Task | null>(null);
  const [elapsedTime, setElapsedTime] = useState<number>(0);
  const eventSourceRef = useRef<EventSource | null>(null);
  const timerRef = useRef<number | null>(null);

  // Webhook相关状态
  const [webhookVisible, setWebhookVisible] = useState(false);
  const [webhookToken, setWebhookToken] = useState<string>('');
  const [currentWebhookTaskId, setCurrentWebhookTaskId] = useState<number | null>(null);

  useEffect(() => {
    loadGroups();
    loadTasks();
    loadRunningTasks();
    loadWebhookToken();
    // 每5秒刷新运行状态
    const interval = setInterval(() => {
      loadRunningTasks();
    }, 5000);

    // 监听窗口大小变化
    const handleResize = () => {
      setIsMobile(window.innerWidth < 768);
    };
    window.addEventListener('resize', handleResize);

    return () => {
      clearInterval(interval);
      window.removeEventListener('resize', handleResize);
      // 清理SSE连接
      if (eventSourceRef.current) {
        eventSourceRef.current.close();
      }
      // 清理计时器
      if (timerRef.current) {
        clearInterval(timerRef.current);
      }
    };
  }, []);

  useEffect(() => {
    loadTasks();
  }, [activeTab]);

  const loadGroups = async () => {
    try {
      const token = localStorage.getItem('token');
      const res = await axios.get('/api/task-groups', {
        headers: { Authorization: `Bearer ${token}` },
      });
      setGroups(res.data);
    } catch (error) {
      console.error('Failed to load groups:', error);
    }
  };

  const loadTasks = async () => {
    setLoading(true);
    try {
      if (activeTab === 'all') {
        const res: any = await taskApi.list();
        setTasks(res);
      } else {
        const groupId = parseInt(activeTab);
        const token = localStorage.getItem('token');
        const res = await axios.get(`/api/task-groups/${groupId}/tasks`, {
          headers: { Authorization: `Bearer ${token}` },
        });
        setTasks(res.data);
      }
    } finally {
      setLoading(false);
    }
  };

  const loadWebhookToken = async () => {
    try {
      const token = localStorage.getItem('token');
      const res = await axios.get('/api/system/webhook-config', {
        headers: { Authorization: `Bearer ${token}` },
      });
      if (res.data.configured && res.data.token) {
        setWebhookToken(res.data.token);
      }
    } catch (error) {
      console.error('Failed to load webhook token:', error);
    }
  };

  const showWebhookUrl = (taskId: number) => {
    setCurrentWebhookTaskId(taskId);
    setWebhookVisible(true);
  };

  const copyWebhookUrl = () => {
    if (currentWebhookTaskId) {
      const url = `${window.location.origin}/api/webhook/tasks/${currentWebhookTaskId}/trigger`;
      navigator.clipboard.writeText(url);
      Message.success('Webhook URL已复制到剪贴板');
    }
  };

  const handleEditGroup = (group: any) => {
    setEditingGroup(group);
    groupForm.setFieldsValue(group);
    setGroupModalVisible(true);
  };

  const handleSubmitGroup = async () => {
    try {
      const values = await groupForm.validate();
      const token = localStorage.getItem('token');

      if (editingGroup) {
        await axios.put(`/api/task-groups/${editingGroup.id}`, values, {
          headers: { Authorization: `Bearer ${token}` },
        });
        Message.success('更新成功');
      } else {
        await axios.post('/api/task-groups', values, {
          headers: { Authorization: `Bearer ${token}` },
        });
        Message.success('创建成功');
      }
      setGroupModalVisible(false);
      loadGroups();
    } catch (error: any) {
      Message.error(error.response?.data?.error || '操作失败');
    }
  };

  const handleDeleteGroup = async (id: number) => {
    try {
      const token = localStorage.getItem('token');
      await axios.delete(`/api/task-groups/${id}`, {
        headers: { Authorization: `Bearer ${token}` },
      });
      Message.success('删除成功');
      if (activeTab === id.toString()) {
        setActiveTab('all');
      }
      loadGroups();
    } catch (error: any) {
      Message.error(error.response?.data?.error || '删除失败');
    }
  };

  const loadRunningTasks = async () => {
    try {
      const res: any = await taskApi.listRunning();
      console.log('运行中任务API返回:', res);
      // API返回的是任务ID数组，不是Task对象数组
      const runningIds = new Set<number>(res);
      console.log('运行中任务ID集合:', Array.from(runningIds));
      setRunningTasks(runningIds);
    } catch (error) {
      console.error('Failed to load running tasks:', error);
    }
  };

  const handleAdd = () => {
    setEditingTask(null);
    form.resetFields();
    form.setFieldsValue({
      type: 'cron',
      enabled: true,
      cron: ['*/5 * * * *'], // 默认每5分钟（5字段格式）
    });
    setVisible(true);
  };

  const handleEdit = (task: Task) => {
    setEditingTask(task);
    // 确保 cron 是数组格式
    const formData = {
      ...task,
      cron: Array.isArray(task.cron) ? task.cron : [task.cron],
    };
    form.setFieldsValue(formData);
    setVisible(true);
  };

  const handleSubmit = async () => {
    try {
      const values = await form.validate();

      // 如果不是定时任务，设置默认的cron表达式
      if (values.type !== 'cron') {
        values.cron = ['0 0 * * *']; // 默认值，不会被使用
      }

      if (editingTask) {
        await taskApi.update(editingTask.id, values);
        Message.success('更新成功');
      } else {
        await taskApi.create(values);
        Message.success('创建成功');
      }
      setVisible(false);
      loadTasks();
    } catch (error: any) {
      Message.error(error.response?.data?.error || '操作失败');
    }
  };

  const handleDelete = async (id: number) => {
    try {
      await taskApi.delete(id);
      Message.success('删除成功');
      loadTasks();
    } catch (error: any) {
      Message.error(error.response?.data?.error || '删除失败');
    }
  };

  const handleRun = async (id: number) => {
    try {
      await taskApi.run(id);
      Message.success('任务已开始执行');
      // 立即刷新状态
      loadRunningTasks();
      loadTasks();
    } catch (error: any) {
      Message.error(error.response?.data?.error || '执行失败');
    }
  };

  const handleKill = async (id: number) => {
    try {
      await taskApi.kill(id);
      Message.success('任务已终止');
      loadRunningTasks();
      loadTasks();
    } catch (error: any) {
      Message.error(error.response?.data?.error || '终止失败');
    }
  };

  const handleViewLog = async (task: Task) => {
    setLogVisible(true);
    setLogContent('');
    setLogLoading(true);
    setCurrentViewTask(task);

    // 清除之前的计时器
    if (timerRef.current) {
      clearInterval(timerRef.current);
      timerRef.current = null;
    }

    // 先刷新运行状态，确保是最新的
    try {
      const res: any = await taskApi.listRunning();
      console.log('查看日志时刷新运行状态，API返回:', res);
      // API返回的是任务ID数组
      const runningIds = new Set(res);
      const isRunning = runningIds.has(task.id);

      console.log('查看日志 - 任务ID:', task.id, '是否运行中:', isRunning, 'runningIds:', Array.from(runningIds));

      if (isRunning) {
        // 实时日志 - 使用SSE
        setIsLiveLog(true);

        // 先获取最近的执行记录
        try {
          const executions: any = await taskApi.listExecutions();
          console.log('执行记录列表:', executions);

          // 找到该任务的执行记录（ExecutionInfo没有status字段，直接找task_id匹配的）
          const currentExecution = executions.find((e: any) => e.task_id === task.id);

          console.log('当前执行记录:', currentExecution);

          if (currentExecution) {
            // 启动实时耗时计时器 - 使用执行记录的开始时间
            const startTimestamp = new Date(currentExecution.started_at).getTime();
            setElapsedTime(Date.now() - startTimestamp);

            timerRef.current = setInterval(() => {
              setElapsedTime(Date.now() - startTimestamp);
            }, 100);

            // 连接SSE获取实时日志
            const token = localStorage.getItem('token');
            const url = `/api/executions/${currentExecution.execution_id}/logs${token ? `?token=${token}` : ''}`;

            console.log('连接SSE:', url);
            setLogContent('[正在连接日志流...]\n');

            const eventSource = new EventSource(url);
            eventSourceRef.current = eventSource;

            // 设置连接超时
            const connectTimeout = setTimeout(() => {
              if (eventSource.readyState === EventSource.CONNECTING) {
                console.warn('SSE连接超时');
                setLogContent(prev => prev + '[连接超时，请检查网络或任务状态]\n');
              }
            }, 5000);

            eventSource.onopen = () => {
              clearTimeout(connectTimeout);
              setLogLoading(false);
              console.log('SSE连接已建立');
              setLogContent(prev => prev.replace('[正在连接日志流...]\n', '[日志流已连接]\n'));
            };

            eventSource.onmessage = (event) => {
              setLogLoading(false);
              console.log('收到日志:', event.data);
              setLogContent(prev => prev + event.data + '\n');
            };

            eventSource.onerror = (error) => {
              clearTimeout(connectTimeout);
              console.error('SSE错误:', error);
              eventSource.close();
              setIsLiveLog(false);
              setLogLoading(false);

              // 停止计时器
              if (timerRef.current) {
                clearInterval(timerRef.current);
                timerRef.current = null;
              }

              // 不要覆盖已有的日志内容
              setLogContent(prev => prev ? prev + '\n[日志流已结束]' : '日志流连接失败');
            };
          } else {
            setLogLoading(false);
            setLogContent('未找到运行中的执行记录');
          }
        } catch (error) {
          console.error('获取执行记录失败:', error);
          setLogLoading(false);
          setLogContent('获取执行记录失败');
        }
      } else {
        // 历史日志 - 获取最后一次执行的日志
        setIsLiveLog(false);

        try {
          const response: any = await logApi.list(task.id, 1);
          setLogLoading(false);

          const logs = response.data || response;
          if (logs && logs.length > 0) {
            const log = logs[0];
            // 添加任务开始时间
            const startTime = new Date(log.created_at).toLocaleString('zh-CN');
            setLogContent(`[任务开始时间: ${startTime}]\n${log.output || '无日志输出'}`);
          } else {
            setLogContent('暂无执行日志');
          }
        } catch (error: any) {
          setLogLoading(false);
          setLogContent('获取日志失败: ' + (error.message || '未知错误'));
        }
      }
    } catch (error) {
      console.error('刷新运行状态失败:', error);
      setLogLoading(false);
      setLogContent('获取运行状态失败');
    }
  };

  const handleCloseLog = () => {
    setLogVisible(false);
    setLogContent('');
    setIsLiveLog(false);
    setCurrentViewTask(null);
    setElapsedTime(0);

    // 关闭SSE连接
    if (eventSourceRef.current) {
      eventSourceRef.current.close();
      eventSourceRef.current = null;
    }

    // 清除计时器
    if (timerRef.current) {
      clearInterval(timerRef.current);
      timerRef.current = null;
    }
  };

  const getTaskTypeTag = (type: string) => {
    const typeMap: Record<string, { color: string; text: string }> = {
      cron: { color: 'blue', text: '定时任务' },
      manual: { color: 'orange', text: '手动任务' },
      startup: { color: 'green', text: '开机任务' },
    };
    const config = typeMap[type] || { color: 'gray', text: type };
    return <Tag color={config.color}>{config.text}</Tag>;
  };

  const columns = [
    {
      title: '任务名称',
      dataIndex: 'name',
      width: isMobile ? 100 : 200,
    },
    {
      title: '类型',
      dataIndex: 'type',
      width: isMobile ? 70 : 120,
      render: (type: string) => getTaskTypeTag(type),
    },
    {
      title: 'Cron表达式',
      dataIndex: 'cron',
      width: isMobile ? 120 : 200,
      render: (cron: string | string[], record: Task) => {
        if (record.type !== 'cron') return '-';
        const cronArray = Array.isArray(cron) ? cron : [cron];
        return (
          <div style={{ fontSize: isMobile ? '10px' : '12px' }}>
            {cronArray.map((c, idx) => (
              <code key={idx} style={{ display: 'block', marginBottom: idx < cronArray.length - 1 ? '4px' : 0 }}>
                {c}
              </code>
            ))}
          </div>
        );
      },
    },
    {
      title: '命令',
      dataIndex: 'command',
      width: isMobile ? 120 : 250,
      ellipsis: true,
      render: (command: string) => (
        <span style={{ fontFamily: 'monospace', fontSize: isMobile ? '10px' : '12px' }}>{command}</span>
      ),
    },
    {
      title: '状态',
      dataIndex: 'enabled',
      width: isMobile ? 70 : 100,
      render: (enabled: boolean, record: Task) => {
        const isRunning = runningTasks.has(record.id);
        return (
          <Space>
            <Tag color={enabled ? 'green' : 'gray'}>
              {enabled ? '启用' : '禁用'}
            </Tag>
            {isRunning && (
              <Tag color="blue" icon={<IconPlayArrow />}>
                {!isMobile && '运行中'}
              </Tag>
            )}
          </Space>
        );
      },
    },
    {
      title: '最后执行',
      dataIndex: 'last_run_at',
      width: isMobile ? 130 : 180,
      render: (time: string, record: Task) => {
        if (!time) return '-';
        const duration = record.last_run_duration
          ? ` (${record.last_run_duration}ms)`
          : '';
        return (
          <div>
            <div style={{ fontSize: isMobile ? '11px' : '14px' }}>
              {new Date(time).toLocaleString('zh-CN')}
            </div>
            {duration && !isMobile && (
              <div style={{ fontSize: 12, color: 'var(--color-text-3)' }}>
                {duration}
              </div>
            )}
          </div>
        );
      },
    },
    {
      title: '下次执行',
      dataIndex: 'next_run_at',
      width: isMobile ? 130 : 180,
      render: (time: string) =>
        time ? (
          <span style={{ fontSize: isMobile ? '11px' : '14px' }}>
            {new Date(time).toLocaleString('zh-CN')}
          </span>
        ) : '-',
    },
    {
      title: '操作',
      width: isMobile ? 100 : 180,
      fixed: 'right' as const,
      render: (_: any, record: Task) => {
        const isRunning = runningTasks.has(record.id);

        const droplist = (
          <Menu>
            <Menu.Item key="edit" onClick={() => handleEdit(record)} disabled={isRunning}>
              <Space>
                <IconEdit />
                编辑
              </Space>
            </Menu.Item>
            {webhookToken && (
              <Menu.Item key="webhook" onClick={() => showWebhookUrl(record.id)}>
                <Space>
                  <IconLink />
                  Webhook
                </Space>
              </Menu.Item>
            )}
            <Menu.Item key="delete" onClick={() => {
              Modal.confirm({
                title: '确定删除此任务吗？',
                onOk: () => handleDelete(record.id),
              });
            }} disabled={isRunning}>
              <Space>
                <IconDelete />
                删除
              </Space>
            </Menu.Item>
          </Menu>
        );

        return (
          <Space>
            {isRunning ? (
              <Popconfirm
                title="确定终止此任务吗？"
                onOk={() => handleKill(record.id)}
              >
                <Button
                  type="text"
                  size="small"
                  status="warning"
                  icon={<IconStop />}
                >
                  {!isMobile && '终止'}
                </Button>
              </Popconfirm>
            ) : (
              <Button
                type="text"
                size="small"
                icon={<IconPlayArrow />}
                onClick={() => handleRun(record.id)}
              >
                {!isMobile && '执行'}
              </Button>
            )}
            <Button
              type="text"
              size="small"
              icon={<IconFile />}
              onClick={() => handleViewLog(record)}
            >
              {!isMobile && '日志'}
            </Button>
            <Dropdown droplist={droplist} position="bl">
              <Button type="text" size="small" icon={<IconMore />} />
            </Dropdown>
          </Space>
        );
      },
    },
  ];

  const renderTabContent = () => {
    return (
      <div>
        <div style={{ marginBottom: 16, display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
          <span style={{ color: '#86909c' }}>
            共 {tasks.length} 个任务
          </span>
          <Button type="primary" icon={<IconPlus />} onClick={handleAdd}>
            新建任务
          </Button>
        </div>
        <Table
          columns={columns}
          data={tasks}
          loading={loading}
          pagination={{ pageSize: 10 }}
          scroll={{ x: isMobile ? 840 : 1200 }}
          rowKey="id"
        />
      </div>
    );
  };

  return (
    <>
      <Card title="任务管理">
        <Tabs
          activeTab={activeTab}
          onChange={setActiveTab}
          type="card"
          extra={
            <Button size="mini" onClick={() => setGroupManageVisible(true)}>
              管理分组
            </Button>
          }
        >
          <TabPane key="all" title="全部任务">
            {renderTabContent()}
          </TabPane>
          {groups.map(group => (
            <TabPane
              key={group.id.toString()}
              title={group.name}
            >
              {renderTabContent()}
            </TabPane>
          ))}
        </Tabs>

      <Modal
        title={editingTask ? '编辑任务' : '新建任务'}
        visible={visible}
        onOk={handleSubmit}
        onCancel={() => setVisible(false)}
        autoFocus={false}
        style={{ width: '90%', maxWidth: 800 }}
      >
        <Form form={form} layout="vertical">
          <Tabs defaultActiveTab="basic">
            <TabPane key="basic" title="基础配置">
              <Row gutter={16}>
                <Col span={12}>
                  <FormItem label="任务名称" field="name" rules={[{ required: true, message: '请输入任务名称' }]}>
                    <Input placeholder="请输入任务名称" />
                  </FormItem>
                </Col>
                <Col span={12}>
                  <FormItem label="任务类型" field="type" rules={[{ required: true }]}>
                    <Select placeholder="请选择任务类型">
                      <Option value="cron">定时任务</Option>
                      <Option value="manual">手动任务</Option>
                      <Option value="startup">开机任务</Option>
                    </Select>
                  </FormItem>
                </Col>
              </Row>

              <Row gutter={16}>
                <Col span={12}>
                  <FormItem label="任务分组" field="group_id">
                    <Select placeholder="请选择分组（可选）" allowClear>
                      {groups.map(group => (
                        <Option key={group.id} value={group.id}>{group.name}</Option>
                      ))}
                    </Select>
                  </FormItem>
                </Col>
              </Row>

              <Form.Item noStyle shouldUpdate>
                {(values) => {
                  const taskType = values.type;
                  return taskType === 'cron' ? (
                    <FormItem
                      label="Cron表达式"
                      field="cron"
                      rules={[
                        {
                          required: true,
                          type: 'array',
                          minLength: 1,
                          message: '请至少添加一个Cron表达式'
                        }
                      ]}
                      extra={
                        <Space size="mini" style={{ fontSize: 12, color: 'var(--color-text-3)' }}>
                          <IconInfoCircle />
                          <span>支持5字段（分 时 日 月 周）或6字段（秒 分 时 日 月 周），例如: */5 * * * * 或 0 */5 * * * *</span>
                        </Space>
                      }
                    >
                      <Form.List field="cron">
                        {(fields, { add, remove }) => (
                          <div>
                            {fields.map((field, index) => (
                              <div key={field.key} style={{ marginBottom: 8 }}>
                                <Space style={{ width: '100%', alignItems: 'flex-start' }}>
                                  <FormItem
                                    field={field.field}
                                    rules={[{ required: true, message: '请输入Cron表达式' }]}
                                    style={{ marginBottom: 0, flex: 1 }}
                                  >
                                    <Input
                                      placeholder="例如: */5 * * * * 或 0 */5 * * * *"
                                      style={{ width: '100%' }}
                                    />
                                  </FormItem>
                                  {fields.length > 1 && (
                                    <Button
                                      type="text"
                                      status="danger"
                                      icon={<IconDelete />}
                                      onClick={() => remove(index)}
                                    />
                                  )}
                                </Space>
                              </div>
                            ))}
                            <Button
                              type="dashed"
                              icon={<IconPlus />}
                              onClick={() => add()}
                              style={{ width: '100%' }}
                            >
                              添加Cron表达式
                            </Button>
                          </div>
                        )}
                      </Form.List>
                    </FormItem>
                  ) : null;
                }}
              </Form.Item>

              <FormItem label="执行命令" field="command" rules={[{ required: true, message: '请输入执行命令' }]}>
                <Input.TextArea
                  placeholder="例如: python3 scripts/test.py&#10;或: node scripts/app.js&#10;或: bash scripts/backup.sh"
                  autoSize={{ minRows: 1, maxRows: 5 }}
                  style={{ fontFamily: 'monospace' }}
                />
              </FormItem>

              <FormItem label="启用" field="enabled" triggerPropName="checked">
                <Switch />
              </FormItem>
            </TabPane>

            <TabPane key="advanced" title="高级配置">
              <FormItem
                label="工作目录"
                field="working_dir"
                extra="命令执行的工作目录。留空则自动根据脚本路径判断；相对路径以scripts目录为基准；支持绝对路径"
              >
                <Input
                  placeholder="例如: git/my-repo 或 /absolute/path"
                  style={{ fontFamily: 'monospace' }}
                />
              </FormItem>

              <FormItem
                label="前置命令"
                field="pre_command"
                extra="在主命令执行前运行，可用于环境准备"
              >
                <Input.TextArea
                  placeholder="例如: cd /path/to/dir"
                  autoSize={{ minRows: 1, maxRows: 5 }}
                  style={{ fontFamily: 'monospace' }}
                />
              </FormItem>

              <FormItem
                label="后置命令"
                field="post_command"
                extra="在主命令执行后运行，可用于清理工作"
              >
                <Input.TextArea
                  placeholder="例如: rm -f /tmp/*.tmp"
                  autoSize={{ minRows: 1, maxRows: 5 }}
                  style={{ fontFamily: 'monospace' }}
                />
              </FormItem>

              <Divider />

              <FormItem
                label="环境变量"
                field="env"
                extra="JSON格式，例如: {&quot;API_KEY&quot;: &quot;xxx&quot;, &quot;DEBUG&quot;: &quot;true&quot;}"
              >
                <Input.TextArea
                  placeholder='{"KEY": "value"}'
                  rows={4}
                  style={{ fontFamily: 'monospace' }}
                />
              </FormItem>
            </TabPane>
          </Tabs>
        </Form>
      </Modal>

      {/* 日志查看弹窗 */}
      <Modal
        title={
          <Space>
            <span>执行日志</span>
            {isLiveLog && <Tag color="blue">实时</Tag>}
          </Space>
        }
        visible={logVisible}
        onCancel={handleCloseLog}
        footer={null}
        style={{ width: '90%', maxWidth: 1000 }}
      >
        <Spin loading={logLoading} style={{ width: '100%' }}>
          <div
            style={{
              background: '#1e1e1e',
              color: '#d4d4d4',
              padding: '16px',
              borderRadius: '4px',
              fontFamily: 'Consolas, Monaco, "Courier New", monospace',
              fontSize: '13px',
              lineHeight: '1.6',
              maxHeight: '500px',
              overflowY: 'auto',
              whiteSpace: 'pre-wrap',
              wordBreak: 'break-all',
            }}
          >
            {logContent || '暂无日志'}
          </div>
          {currentViewTask && runningTasks.has(currentViewTask.id) && (
            <div
              style={{
                marginTop: '12px',
                padding: '8px 12px',
                background: '#f0f9ff',
                border: '1px solid #bae7ff',
                borderRadius: '4px',
                display: 'flex',
                alignItems: 'center',
                gap: '8px',
              }}
            >
              <span style={{ color: '#1890ff', fontWeight: 500 }}>
                实时耗时: {elapsedTime}ms ({(elapsedTime / 1000).toFixed(2)}s)
              </span>
              <IconPlayArrow style={{ color: '#1890ff', fontSize: '16px', animation: 'spin 1s linear infinite' }} />
            </div>
          )}
        </Spin>
      </Modal>

      <style>{`
        @keyframes spin {
          from { transform: rotate(0deg); }
          to { transform: rotate(360deg); }
        }
      `}</style>

      <Modal
        title="分组管理"
        visible={groupManageVisible}
        onCancel={() => setGroupManageVisible(false)}
        footer={null}
        style={{ width: '90%', maxWidth: 600 }}
      >
        <div style={{ marginBottom: 16 }}>
          <Button type="primary" icon={<IconPlus />} onClick={() => {
            setEditingGroup(null);
            groupForm.resetFields();
            setGroupModalVisible(true);
          }}>
            新建分组
          </Button>
        </div>
        <Table
          columns={[
            {
              title: '分组名称',
              dataIndex: 'name',
            },
            {
              title: '描述',
              dataIndex: 'description',
            },
            {
              title: '创建时间',
              dataIndex: 'created_at',
              render: (time: string) => new Date(time).toLocaleString('zh-CN'),
            },
            {
              title: '操作',
              width: 120,
              render: (_: any, record: any) => (
                <Space size="mini">
                  <Button
                    type="text"
                    size="mini"
                    icon={<IconEdit />}
                    onClick={() => handleEditGroup(record)}
                  />
                  <Popconfirm
                    title="确定删除此分组吗？"
                    onOk={() => handleDeleteGroup(record.id)}
                  >
                    <Button
                      type="text"
                      size="mini"
                      status="danger"
                      icon={<IconDelete />}
                    />
                  </Popconfirm>
                </Space>
              ),
            },
          ]}
          data={groups}
          pagination={false}
          rowKey="id"
        />
      </Modal>

      <Modal
        title={editingGroup ? '编辑分组' : '新建分组'}
        visible={groupModalVisible}
        onOk={handleSubmitGroup}
        onCancel={() => setGroupModalVisible(false)}
        autoFocus={false}
        style={{ width: '90%', maxWidth: 500 }}
      >
        <Form form={groupForm} layout="vertical">
          <FormItem label="分组名称" field="name" rules={[{ required: true, message: '请输入分组名称' }]}>
            <Input placeholder="请输入分组名称" />
          </FormItem>
          <FormItem label="分组描述" field="description">
            <Input.TextArea placeholder="请输入分组描述" rows={3} />
          </FormItem>
        </Form>
      </Modal>

      <Modal
        title="Webhook URL"
        visible={webhookVisible}
        onCancel={() => setWebhookVisible(false)}
        footer={
          <Space>
            <Button onClick={() => setWebhookVisible(false)}>关闭</Button>
            <Button type="primary" onClick={copyWebhookUrl}>复制URL</Button>
          </Space>
        }
        style={{ width: '90%', maxWidth: 600 }}
      >
        <Space direction="vertical" style={{ width: '100%' }} size="large">
          <div>
            <Typography.Text bold>Webhook URL:</Typography.Text>
            <Input.TextArea
              value={currentWebhookTaskId ? `${window.location.origin}/api/webhook/tasks/${currentWebhookTaskId}/trigger` : ''}
              readOnly
              autoSize={{ minRows: 2, maxRows: 4 }}
              style={{ marginTop: 8 }}
            />
          </div>
          <div>
            <Typography.Text bold>使用方法:</Typography.Text>
            <Typography.Paragraph style={{ marginTop: 8 }}>
              使用POST请求调用此URL，需要在请求头中添加：
              <pre style={{ background: 'var(--color-fill-2)', padding: '8px', borderRadius: '4px', marginTop: 8 }}>
                Authorization: Bearer {webhookToken}
              </pre>
            </Typography.Paragraph>
          </div>
          <div>
            <Typography.Text bold>示例:</Typography.Text>
            <pre style={{ background: 'var(--color-fill-2)', padding: '12px', borderRadius: '4px', marginTop: 8, overflow: 'auto' }}>
{`curl -X POST \\
  ${currentWebhookTaskId ? `${window.location.origin}/api/webhook/tasks/${currentWebhookTaskId}/trigger` : ''} \\
  -H "Authorization: Bearer ${webhookToken}"`}
            </pre>
          </div>
        </Space>
      </Modal>
    </Card>
    </>
  );
};

export default Tasks;
