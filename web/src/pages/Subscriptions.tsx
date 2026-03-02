import React, { useEffect, useState } from 'react';
import {
  Card,
  Table,
  Button,
  Space,
  Modal,
  Form,
  Input,
  Message,
  Popconfirm,
  Tag,
  Switch,
} from '@arco-design/web-react';
import { IconPlus, IconPlayArrow, IconEdit, IconDelete, IconFile } from '@arco-design/web-react/icon';
import { subscriptionApi } from '@/api/subscription';
import type { Subscription } from '@/types';

const FormItem = Form.Item;

const Subscriptions: React.FC = () => {
  const [subscriptions, setSubscriptions] = useState<Subscription[]>([]);
  const [loading, setLoading] = useState(false);
  const [visible, setVisible] = useState(false);
  const [logVisible, setLogVisible] = useState(false);
  const [logContent, setLogContent] = useState('');
  const [editingSubscription, setEditingSubscription] = useState<Subscription | null>(null);
  const [form] = Form.useForm();

  useEffect(() => {
    loadSubscriptions(true);
    const interval = setInterval(() => {
      loadSubscriptions(false);
    }, 5000);
    return () => clearInterval(interval);
  }, []);

  const loadSubscriptions = async (showLoading: boolean = true) => {
    if (showLoading) {
      setLoading(true);
    }
    try {
      const res = await subscriptionApi.list();
      setSubscriptions(res);
    } catch (error: any) {
      Message.error(error.response?.data?.error || '加载失败');
    } finally {
      if (showLoading) {
        setLoading(false);
      }
    }
  };

  const handleAdd = () => {
    setEditingSubscription(null);
    form.resetFields();
    form.setFieldsValue({
      branch: 'main',
      schedule: '0 0 * * *',
      enabled: true,
    });
    setVisible(true);
  };

  const handleEdit = (record: Subscription) => {
    setEditingSubscription(record);
    form.setFieldsValue(record);
    setVisible(true);
  };

  const handleSubmit = async () => {
    try {
      const values = await form.validate();
      if (editingSubscription) {
        await subscriptionApi.update(editingSubscription.id, values);
        Message.success('更新成功');
      } else {
        await subscriptionApi.create(values);
        Message.success('创建成功');
      }
      setVisible(false);
      loadSubscriptions(false);
    } catch (error: any) {
      Message.error(error.response?.data?.error || '操作失败');
    }
  };

  const handleDelete = async (id: number) => {
    try {
      await subscriptionApi.delete(id);
      Message.success('删除成功');
      loadSubscriptions(false);
    } catch (error: any) {
      Message.error(error.response?.data?.error || '删除失败');
    }
  };

  const handleRun = async (id: number) => {
    try {
      await subscriptionApi.run(id);
      Message.success('已开始拉取，请稍后查看状态');
      // 开始轮询状态
      const pollInterval = setInterval(async () => {
        const subs = await subscriptionApi.list();
        const sub = subs.find((s: any) => s.id === id);
        if (sub && sub.last_run_status !== 'running') {
          clearInterval(pollInterval);
          loadSubscriptions(false);
          if (sub.last_run_status === 'success') {
            Message.success('拉取成功');
          } else if (sub.last_run_status === 'failed') {
            Message.error('拉取失败，请查看日志');
          }
        }
      }, 2000);
      // 30秒后停止轮询
      setTimeout(() => clearInterval(pollInterval), 30000);
    } catch (error: any) {
      Message.error(error.response?.data?.error || '启动失败');
    }
  };

  const handleToggleEnabled = async (id: number, enabled: boolean) => {
    try {
      await subscriptionApi.update(id, { enabled });
      Message.success(enabled ? '已启用' : '已禁用');
      loadSubscriptions(false);
    } catch (error: any) {
      Message.error(error.response?.data?.error || '操作失败');
    }
  };

  const handleViewLog = (record: Subscription) => {
    setLogContent(record.last_run_log || '暂无日志');
    setLogVisible(true);
  };

  const getStatusTag = (status?: string) => {
    if (!status) return <Tag color="gray">未运行</Tag>;
    if (status === 'success') return <Tag color="green">成功</Tag>;
    if (status === 'failed') return <Tag color="red">失败</Tag>;
    if (status === 'running') return <Tag color="blue">运行中</Tag>;
    return <Tag color="gray">{status}</Tag>;
  };

  const columns = [
    {
      title: '名称',
      dataIndex: 'name',
      width: 150,
    },
    {
      title: '仓库地址',
      dataIndex: 'url',
      ellipsis: true,
    },
    {
      title: '分支',
      dataIndex: 'branch',
      width: 100,
    },
    {
      title: '定时规则',
      dataIndex: 'schedule',
      width: 150,
    },
    {
      title: '状态',
      dataIndex: 'enabled',
      width: 80,
      render: (enabled: boolean, record: Subscription) => (
        <Switch
          checked={enabled}
          onChange={(checked) => handleToggleEnabled(record.id, checked)}
        />
      ),
    },
    {
      title: '最后运行',
      dataIndex: 'last_run_status',
      width: 100,
      render: (status: string) => getStatusTag(status),
    },
    {
      title: '最后运行时间',
      dataIndex: 'last_run_time',
      width: 180,
      render: (time: string) => time ? new Date(time).toLocaleString('zh-CN') : '-',
    },
    {
      title: '操作',
      width: 200,
      render: (_: any, record: Subscription) => (
        <Space>
          <Button
            type="text"
            size="mini"
            icon={<IconPlayArrow />}
            onClick={() => handleRun(record.id)}
            title="立即运行"
          />
          <Button
            type="text"
            size="mini"
            icon={<IconFile />}
            onClick={() => handleViewLog(record)}
            disabled={!record.last_run_log}
            title="查看日志"
          />
          <Button
            type="text"
            size="mini"
            icon={<IconEdit />}
            onClick={() => handleEdit(record)}
            title="编辑"
          />
          <Popconfirm
            title="确定删除此订阅吗？"
            onOk={() => handleDelete(record.id)}
          >
            <Button
              type="text"
              size="mini"
              status="danger"
              icon={<IconDelete />}
              title="删除"
            />
          </Popconfirm>
        </Space>
      ),
    },
  ];

  return (
    <>
      <Card
        title="订阅管理"
        extra={
          <Button type="primary" icon={<IconPlus />} onClick={handleAdd}>
            添加订阅
          </Button>
        }
      >
        <Table
          columns={columns}
          data={subscriptions}
          loading={loading}
          pagination={{ pageSize: 10 }}
          rowKey="id"
        />
      </Card>

      <Modal
        title={editingSubscription ? '编辑订阅' : '添加订阅'}
        visible={visible}
        onOk={handleSubmit}
        onCancel={() => setVisible(false)}
        autoFocus={false}
        style={{ width: '90%', maxWidth: 600 }}
      >
        <Form form={form} layout="vertical">
          <FormItem label="订阅名称" field="name" rules={[{ required: true, message: '请输入订阅名称' }]}>
            <Input placeholder="例如: 京东脚本库" />
          </FormItem>
          <FormItem label="仓库地址" field="url" rules={[{ required: true, message: '请输入仓库地址' }]}>
            <Input placeholder="https://github.com/user/repo.git" />
          </FormItem>
          <FormItem label="分支" field="branch" rules={[{ required: true, message: '请输入分支名' }]}>
            <Input placeholder="main" />
          </FormItem>
          <FormItem label="定时规则" field="schedule" rules={[{ required: true, message: '请输入定时规则' }]}>
            <Input placeholder="0 0 * * * (每天0点)" />
          </FormItem>
          <FormItem label="启用" field="enabled" triggerPropName="checked">
            <Switch />
          </FormItem>
        </Form>
      </Modal>

      <Modal
        title="运行日志"
        visible={logVisible}
        onCancel={() => setLogVisible(false)}
        footer={null}
        style={{ width: '90%', maxWidth: 800 }}
      >
        <pre style={{
          background: '#1e1e1e',
          color: '#d4d4d4',
          padding: 16,
          borderRadius: 4,
          maxHeight: 500,
          overflow: 'auto',
          fontSize: 12,
          lineHeight: 1.5,
        }}>
          {logContent}
        </pre>
      </Modal>
    </>
  );
};

export default Subscriptions;
