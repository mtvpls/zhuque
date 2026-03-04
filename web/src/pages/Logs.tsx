import React, { useEffect, useState } from 'react';
import {
  Card,
  Table,
  Select,
  Space,
  Tag,
  Button,
  Modal,
  Spin,
} from '@arco-design/web-react';
import { IconFile, IconRefresh } from '@arco-design/web-react/icon';
import { logApi } from '@/api/log';
import { taskApi } from '@/api/task';
import type { Log, Task } from '@/types';

const { Option } = Select;

const Logs: React.FC = () => {
  console.log('Logs component mounted');
  const [logs, setLogs] = useState<Log[]>([]);
  const [tasks, setTasks] = useState<Task[]>([]);
  const [loading, setLoading] = useState(false);
  const [selectedTaskId, setSelectedTaskId] = useState<number | null>(null);
  const [logVisible, setLogVisible] = useState(false);
  const [logContent, setLogContent] = useState('');
  const [logLoading, setLogLoading] = useState(false);
  const [pagination, setPagination] = useState({
    current: 1,
    pageSize: 10,
    total: 0,
  });

  useEffect(() => {
    console.log('useEffect for loadTasks triggered');
    loadTasks();
  }, []);

  useEffect(() => {
    console.log('selectedTaskId changed:', selectedTaskId);
    if (selectedTaskId) {
      console.log('Calling loadLogs with taskId:', selectedTaskId);
      loadLogs(selectedTaskId, 1);
    }
  }, [selectedTaskId]);

  const loadTasks = async () => {
    try {
      const res: any = await taskApi.list();
      console.log('Tasks loaded:', res);
      setTasks(res);
      if (res.length > 0) {
        console.log('Setting selectedTaskId to:', res[0].id);
        setSelectedTaskId(res[0].id);
      } else {
        console.log('No tasks found');
      }
    } catch (error) {
      console.error('Failed to load tasks:', error);
    }
  };

  const loadLogs = async (taskId: number, page = 1) => {
    setLoading(true);
    try {
      const res: any = await logApi.list(taskId, page, pagination.pageSize);
      console.log('Logs API response:', res);
      console.log('Logs data:', res.data);
      console.log('Logs data length:', res.data?.length);
      setLogs(res.data || []);
      setPagination({
        current: res.page || 1,
        pageSize: res.page_size || 10,
        total: res.total || 0,
      });
    } catch (error) {
      console.error('Failed to load logs:', error);
      setLogs([]);
    } finally {
      setLoading(false);
    }
  };

  const handlePageChange = (page: number) => {
    if (selectedTaskId) {
      loadLogs(selectedTaskId, page);
    }
  };

  const handleViewLog = async (log: Log) => {
    setLogVisible(true);
    setLogContent('');
    setLogLoading(true);

    try {
      const startTime = new Date(log.created_at).toLocaleString('zh-CN');
      setLogContent(`[任务开始时间: ${startTime}]\n${log.output || '无日志输出'}`);
    } finally {
      setLogLoading(false);
    }
  };

  const columns = [
    {
      title: '任务名称',
      dataIndex: 'task_id',
      width: 200,
      render: (taskId: number) => {
        const task = tasks.find(t => t.id === taskId);
        return task?.name || `任务 ${taskId}`;
      },
    },
    {
      title: '状态',
      dataIndex: 'status',
      width: 100,
      render: (status: string) => {
        const statusMap: Record<string, { color: string; text: string }> = {
          success: { color: 'green', text: '成功' },
          failed: { color: 'red', text: '失败' },
          running: { color: 'blue', text: '运行中' },
        };
        const config = statusMap[status] || { color: 'gray', text: status };
        return <Tag color={config.color}>{config.text}</Tag>;
      },
    },
    {
      title: '执行时间',
      dataIndex: 'created_at',
      width: 180,
      render: (time: string) => new Date(time).toLocaleString('zh-CN'),
    },
    {
      title: '耗时',
      dataIndex: 'duration',
      width: 120,
      render: (duration: number | undefined) => {
        if (!duration) return '-';
        return (
          <span style={{ fontFamily: 'monospace', fontSize: '12px' }}>
            {duration}ms ({(duration / 1000).toFixed(2)}s)
          </span>
        );
      },
    },
    {
      title: '日志预览',
      dataIndex: 'output',
      ellipsis: true,
      render: (output: string) => (
        <span style={{ fontFamily: 'monospace', fontSize: '12px' }}>
          {output ? output.substring(0, 100) : '无输出'}
        </span>
      ),
    },
    {
      title: '操作',
      width: 120,
      fixed: 'right' as const,
      render: (_: any, record: Log) => (
        <Button
          type="text"
          size="small"
          icon={<IconFile />}
          onClick={() => handleViewLog(record)}
        >
          查看日志
        </Button>
      ),
    },
  ];

  return (
    <Card
      title="执行日志"
      extra={
        <Space>
          <Select
            placeholder="选择任务"
            style={{ width: 200 }}
            value={selectedTaskId ?? undefined}
            onChange={(value) => setSelectedTaskId(value as number)}
          >
            {tasks.map((task) => (
              <Option key={task.id} value={task.id}>
                {task.name}
              </Option>
            ))}
          </Select>
          <Button
            icon={<IconRefresh />}
            onClick={() => selectedTaskId && loadLogs(selectedTaskId, pagination.current)}
          >
            刷新
          </Button>
        </Space>
      }
    >
      <Table
        columns={columns}
        data={logs}
        loading={loading}
        pagination={{
          current: pagination.current,
          pageSize: pagination.pageSize,
          total: pagination.total,
          onChange: handlePageChange,
        }}
        scroll={{ x: 1000 }}
        rowKey="id"
      />

      <Modal
        title="执行日志"
        visible={logVisible}
        onCancel={() => setLogVisible(false)}
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
        </Spin>
      </Modal>
    </Card>
  );
};

export default Logs;
