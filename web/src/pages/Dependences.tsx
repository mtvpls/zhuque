import React, { useEffect, useState } from 'react';
import {
  Card,
  Table,
  Button,
  Space,
  Modal,
  Form,
  Input,
  Select,
  Message,
  Popconfirm,
  Tag,
  Spin,
  Tabs,
} from '@arco-design/web-react';
import { IconPlus, IconRefresh, IconDelete, IconFile } from '@arco-design/web-react/icon';
import { dependenceApi } from '@/api/dependence';
import type { Dependence } from '@/types';

const FormItem = Form.Item;
const { Option } = Select;
const TabPane = Tabs.TabPane;

const Dependences: React.FC = () => {
  const [dependences, setDependences] = useState<Dependence[]>([]);
  const [loading, setLoading] = useState(false);
  const [visible, setVisible] = useState(false);
  const [logVisible, setLogVisible] = useState(false);
  const [logContent, setLogContent] = useState('');
  const [logLoading, setLogLoading] = useState(false);
  const [activeTab, setActiveTab] = useState<string>('python');
  const [form] = Form.useForm();

  useEffect(() => {
    loadDependences(true);
    // 每5秒刷新一次状态
    const interval = setInterval(() => {
      loadDependences(false);
    }, 5000);
    return () => clearInterval(interval);
  }, []);

  const loadDependences = async (showLoading: boolean = true) => {
    if (showLoading) {
      setLoading(true);
    }
    try {
      const res: any = await dependenceApi.list();
      setDependences(res);
    } finally {
      if (showLoading) {
        setLoading(false);
      }
    }
  };

  const getFilteredDependences = (type: number) => {
    return dependences.filter(dep => dep.dep_type === type);
  };

  const handleAdd = (depType: string) => {
    form.resetFields();
    form.setFieldsValue({ dep_type: depType });
    setVisible(true);
  };

  const handleSubmit = async () => {
    try {
      const values = await form.validate();
      // 按换行符分割依赖名称，过滤空行
      const names = values.name
        .split('\n')
        .map((n: string) => n.trim())
        .filter((n: string) => n.length > 0);

      if (names.length === 0) {
        Message.warning('请输入至少一个依赖名称');
        return;
      }

      // 构建批量创建的 payload
      const payloads = names.map((name: string) => ({
        name,
        type: values.dep_type,
        remark: values.remark,
      }));

      if (payloads.length === 1) {
        // 单个依赖使用单个创建接口
        await dependenceApi.create(payloads[0]);
        Message.success('添加成功，正在安装...');
      } else {
        // 多个依赖使用批量创建接口
        await dependenceApi.createBatch(payloads);
        Message.success(`已添加 ${payloads.length} 个依赖，正在安装...`);
      }

      setVisible(false);
      loadDependences(false);
    } catch (error: any) {
      Message.error(error.response?.data?.error || '操作失败');
    }
  };

  const handleDelete = async (id: number) => {
    try {
      await dependenceApi.delete(id);
      Message.success('删除成功');
      loadDependences(false);
    } catch (error: any) {
      Message.error(error.response?.data?.error || '删除失败');
    }
  };

  const handleReinstall = async (id: number) => {
    try {
      await dependenceApi.reinstall(id);
      Message.success('重新安装中...');
      loadDependences(false);
    } catch (error: any) {
      Message.error(error.response?.data?.error || '操作失败');
    }
  };

  const handleViewLog = (dep: Dependence) => {
    setLogVisible(true);
    setLogLoading(true);
    setLogContent('');

    try {
      // 解析日志 JSON
      const logs = dep.log ? JSON.parse(dep.log) : [];
      setLogContent(logs.join('\n') || '暂无日志');
    } catch (error) {
      setLogContent('日志解析失败');
    } finally {
      setLogLoading(false);
    }
  };

  const getStatusTag = (status: number) => {
    const statusMap: Record<number, { color: string; text: string }> = {
      0: { color: 'blue', text: '安装中' },
      1: { color: 'green', text: '已安装' },
      2: { color: 'red', text: '失败' },
      3: { color: 'orange', text: '卸载中' },
      4: { color: 'gray', text: '已卸载' },
    };
    const config = statusMap[status] || { color: 'gray', text: '未知' };
    return <Tag color={config.color}>{config.text}</Tag>;
  };

  const columns = [
    {
      title: '依赖名称',
      dataIndex: 'name',
      width: 200,
      render: (name: string) => (
        <span style={{ fontFamily: 'monospace', fontWeight: 'bold' }}>{name}</span>
      ),
    },
    {
      title: '状态',
      dataIndex: 'status',
      width: 120,
      render: (status: number) => getStatusTag(status),
    },
    {
      title: '备注',
      dataIndex: 'remark',
      ellipsis: true,
    },
    {
      title: '创建时间',
      dataIndex: 'created_at',
      width: 180,
      render: (time: string) => new Date(time).toLocaleString('zh-CN'),
    },
    {
      title: '操作',
      width: 120,
      fixed: 'right' as const,
      render: (_: any, record: Dependence) => {
        const isInstalling = record.status === 0 || record.status === 3;
        return (
          <Space size="mini">
            <Button
              type="text"
              size="mini"
              icon={<IconFile />}
              onClick={() => handleViewLog(record)}
              title="查看日志"
            />
            <Button
              type="text"
              size="mini"
              icon={<IconRefresh />}
              onClick={() => handleReinstall(record.id)}
              disabled={isInstalling}
              title="重新安装"
            />
            <Popconfirm
              title="确定删除此依赖吗？"
              onOk={() => handleDelete(record.id)}
            >
              <Button
                type="text"
                size="mini"
                status="danger"
                icon={<IconDelete />}
                disabled={isInstalling}
                title="删除"
              />
            </Popconfirm>
          </Space>
        );
      },
    },
  ];

  const renderTabContent = (type: number, typeName: string, typeValue: string) => {
    const filteredData = getFilteredDependences(type);
    return (
      <div>
        <div style={{ marginBottom: 16, display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
          <span style={{ color: '#86909c' }}>
            共 {filteredData.length} 个依赖
          </span>
          <Button
            type="primary"
            icon={<IconPlus />}
            onClick={() => handleAdd(typeValue)}
          >
            添加{typeName}依赖
          </Button>
        </div>
        <Table
          columns={columns}
          data={filteredData}
          loading={loading}
          pagination={{ pageSize: 10 }}
          rowKey="id"
        />
      </div>
    );
  };

  return (
    <>
      <Card title="依赖管理">
        <Tabs activeTab={activeTab} onChange={setActiveTab} type="card">
          <TabPane key="python" title="Python">
            {renderTabContent(1, 'Python', 'python')}
          </TabPane>
          <TabPane key="nodejs" title="Node.js">
            {renderTabContent(0, 'Node.js', 'nodejs')}
          </TabPane>
          <TabPane key="linux" title="Linux">
            {renderTabContent(2, 'Linux', 'linux')}
          </TabPane>
        </Tabs>

        <Modal
          title="添加依赖"
          visible={visible}
          onOk={handleSubmit}
          onCancel={() => setVisible(false)}
          autoFocus={false}
          style={{ width: '90%', maxWidth: 600 }}
        >
          <Form form={form} layout="vertical">
            <FormItem label="依赖类型" field="dep_type" rules={[{ required: true }]}>
              <Select placeholder="请选择依赖类型">
                <Option value="python">Python (pip)</Option>
                <Option value="nodejs">Node.js (npm)</Option>
                <Option value="linux">Linux (apt)</Option>
              </Select>
            </FormItem>
            <FormItem label="依赖名称" field="name" rules={[{ required: true, message: '请输入依赖名称' }]}>
              <Input.TextArea
                placeholder="例如: requests&#10;可输入多个依赖，每行一个"
                autoSize={{ minRows: 3, maxRows: 10 }}
              />
            </FormItem>
            <FormItem label="备注" field="remark">
              <Input placeholder="依赖说明" />
            </FormItem>
          </Form>
        </Modal>
      </Card>

      <Modal
        title="安装日志"
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
    </>
  );
};

export default Dependences;
