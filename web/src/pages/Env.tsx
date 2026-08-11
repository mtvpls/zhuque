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
  Switch,
} from '@arco-design/web-react';
import { IconPlus, IconEdit, IconDelete, IconImport } from '@arco-design/web-react/icon';
import { envApi } from '@/api/env';
import type { EnvVar } from '@/types';
import MobileRecordCard from '@/components/MobileRecordCard';

const FormItem = Form.Item;

const parseDotEnvContent = (content: string): Array<{ key: string; value: string }> => {
  const result: Array<{ key: string; value: string }> = [];
  for (const line of content.split('\n')) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith('#')) continue;
    const eqIdx = trimmed.indexOf('=');
    if (eqIdx === -1) continue;
    const key = trimmed.substring(0, eqIdx).trim();
    let value = trimmed.substring(eqIdx + 1);
    if (
      (value.startsWith('"') && value.endsWith('"')) ||
      (value.startsWith("'") && value.endsWith("'"))
    ) {
      value = value.slice(1, -1);
    }
    if (key) result.push({ key, value });
  }
  return result;
};

const Env: React.FC = () => {
  const [envVars, setEnvVars] = useState<EnvVar[]>([]);
  const [loading, setLoading] = useState(false);
  const [visible, setVisible] = useState(false);
  const [editingEnv, setEditingEnv] = useState<EnvVar | null>(null);
  const [searchInput, setSearchInput] = useState('');
  const [searchKeyword, setSearchKeyword] = useState('');
  const [isMobile, setIsMobile] = useState(window.innerWidth < 768);
  const [form] = Form.useForm();

  // 批量导入
  const [batchVisible, setBatchVisible] = useState(false);
  const [batchContent, setBatchContent] = useState('');
  const [allowOverwrite, setAllowOverwrite] = useState(false);
  const [batchLoading, setBatchLoading] = useState(false);

  // 批量删除
  const [selectedRowKeys, setSelectedRowKeys] = useState<number[]>([]);
  const [batchDeleteLoading, setBatchDeleteLoading] = useState(false);

  useEffect(() => {
    const handleResize = () => {
      setIsMobile(window.innerWidth < 768);
    };
    window.addEventListener('resize', handleResize);
    return () => {
      window.removeEventListener('resize', handleResize);
    };
  }, []);

  useEffect(() => {
    loadEnvVars();
  }, [searchKeyword]);

  const loadEnvVars = async () => {
    setLoading(true);
    try {
      const search = searchKeyword.trim() || undefined;
      const res: any = await envApi.list(search ? { search } : undefined);
      setEnvVars(res);
    } finally {
      setLoading(false);
    }
  };

  const handleAdd = () => {
    setEditingEnv(null);
    form.resetFields();
    setVisible(true);
  };

  const handleEdit = (env: EnvVar) => {
    setEditingEnv(env);
    form.setFieldsValue(env);
    setVisible(true);
  };

  const handleSubmit = async () => {
    try {
      const values = await form.validate();

      if (editingEnv) {
        await envApi.update(editingEnv.id, values);
        Message.success('更新成功');
      } else {
        await envApi.create(values);
        Message.success('创建成功');
      }
      setVisible(false);
      loadEnvVars();
    } catch (error: any) {
      Message.error(error.response?.data?.error || '操作失败');
    }
  };

  const handleDelete = async (id: number) => {
    try {
      await envApi.delete(id);
      Message.success('删除成功');
      loadEnvVars();
    } catch (error: any) {
      Message.error(error.response?.data?.error || '删除失败');
    }
  };

  const handleBatchImport = async () => {
    const pairs = parseDotEnvContent(batchContent);
    if (pairs.length === 0) {
      Message.warning('未找到有效的环境变量，请检查格式');
      return;
    }
    setBatchLoading(true);
    try {
      const res = await envApi.batchImport(pairs, allowOverwrite);
      const { created, updated } = res;
      const parts = [];
      if (created > 0) parts.push(`新增 ${created} 个`);
      if (updated > 0) parts.push(`更新 ${updated} 个`);
      Message.success(`导入完成：${parts.join('，')}`);
      setBatchVisible(false);
      setBatchContent('');
      setAllowOverwrite(false);
      loadEnvVars();
    } catch (error: any) {
      Message.error(error.response?.data?.error || '批量导入失败');
    } finally {
      setBatchLoading(false);
    }
  };

  const handleBatchDelete = async () => {
    if (selectedRowKeys.length === 0) return;
    setBatchDeleteLoading(true);
    try {
      const res: any = await envApi.batchDelete(selectedRowKeys);
      Message.success(`已删除 ${res.deleted} 个环境变量`);
      setSelectedRowKeys([]);
      loadEnvVars();
    } catch (error: any) {
      Message.error(error.response?.data?.error || '批量删除失败');
    } finally {
      setBatchDeleteLoading(false);
    }
  };

  const renderMobileCard = (env: EnvVar) => (
    <MobileRecordCard
      key={env.id}
      eyebrow={env.remark || '环境变量'}
      title={<code>{env.key}</code>}
      status={(
        <Switch
          size="small"
          checked={env.enabled}
          onChange={async (checked) => {
            try {
              await envApi.update(env.id, { enabled: checked });
              Message.success(checked ? '已启用' : '已禁用');
              loadEnvVars();
            } catch (error: any) {
              Message.error(error.response?.data?.error || '操作失败');
            }
          }}
        />
      )}
      fields={[
        { label: '变量值', value: <code title={env.value}>{env.value}</code>, wide: true },
        { label: '创建时间', value: new Date(env.created_at).toLocaleString('zh-CN') },
        { label: '状态', value: env.enabled ? '已启用' : '已禁用' },
      ]}
      actions={(
        <>
          <Button type="secondary" size="small" icon={<IconEdit />} onClick={() => handleEdit(env)}>编辑</Button>
          <Popconfirm title="确定删除此环境变量吗？" onOk={() => handleDelete(env.id)}>
            <Button type="text" size="small" status="danger" icon={<IconDelete />}>删除</Button>
          </Popconfirm>
        </>
      )}
    />
  );

  const columns = [
    {
      title: '变量名',
      dataIndex: 'key',
      width: 200,
      render: (key: string) => (
        <span style={{ fontFamily: 'monospace', fontWeight: 'bold' }}>{key}</span>
      ),
    },
    {
      title: '变量值',
      dataIndex: 'value',
      width: 300,
      ellipsis: true,
      render: (value: string) => (
        <span style={{ fontFamily: 'monospace', fontSize: '12px' }}>{value}</span>
      ),
    },
    {
      title: '描述',
      dataIndex: 'remark',
      width: 100,
      ellipsis: true,
    },
    {
      title: '状态',
      dataIndex: 'enabled',
      width: 100,
      render: (enabled: boolean, record: EnvVar) => (
        <Switch
          checked={enabled}
          onChange={async (checked) => {
            try {
              await envApi.update(record.id, { enabled: checked });
              Message.success(checked ? '已启用' : '已禁用');
              loadEnvVars();
            } catch (error: any) {
              Message.error(error.response?.data?.error || '操作失败');
            }
          }}
        />
      ),
    },
    {
      title: '创建时间',
      dataIndex: 'created_at',
      width: 180,
      render: (time: string) => new Date(time).toLocaleString('zh-CN'),
    },
    {
      title: '操作',
      width: 150,
      render: (_: any, record: EnvVar) => (
        <Space>
          <Button
            type="text"
            size="small"
            icon={<IconEdit />}
            onClick={() => handleEdit(record)}
          >
            编辑
          </Button>
          <Popconfirm
            title="确定删除此环境变量吗？"
            onOk={() => handleDelete(record.id)}
          >
            <Button type="text" size="small" status="danger" icon={<IconDelete />}>
              删除
            </Button>
          </Popconfirm>
        </Space>
      ),
    },
  ];

  return (
    <Card title="环境变量">
      <div
        style={{
          marginBottom: 16,
          display: 'flex',
          flexDirection: isMobile ? 'column' : 'row',
          justifyContent: isMobile ? 'flex-start' : 'space-between',
          alignItems: isMobile ? 'flex-end' : 'center',
          gap: 12,
        }}
      >
        <Space>
          <Button type="primary" size={isMobile ? 'small' : 'default'} icon={<IconPlus />} onClick={handleAdd}>
            {isMobile ? null : '新建变量'}
          </Button>
          <Button size={isMobile ? 'small' : 'default'} icon={<IconImport />} onClick={() => setBatchVisible(true)}>
            {isMobile ? null : '批量导入'}
          </Button>
          {selectedRowKeys.length > 0 && (
            <Popconfirm
              title={`确定删除选中的 ${selectedRowKeys.length} 个环境变量吗？`}
              onOk={handleBatchDelete}
            >
              <Button
                size={isMobile ? 'small' : 'default'}
                status="danger"
                icon={<IconDelete />}
                loading={batchDeleteLoading}
              >
                {isMobile ? `(${selectedRowKeys.length})` : `批量删除 (${selectedRowKeys.length})`}
              </Button>
            </Popconfirm>
          )}
        </Space>
        <Input.Search
          allowClear
          placeholder="搜索变量名 / 值 / 描述，回车搜索"
          value={searchInput}
          onChange={(v) => {
            setSearchInput(v);
            if (v === '' && searchKeyword !== '') {
              setSearchKeyword('');
            }
          }}
          onSearch={(v) => setSearchKeyword(v.trim())}
          style={{ width: isMobile ? 260 : 260, maxWidth: '100%' }}
        />
      </div>
      {isMobile ? (
        <div className="mobile-record-list">
          {loading ? (
            <div className="mobile-record-list__loading">加载中...</div>
          ) : envVars.length > 0 ? (
            envVars.map(renderMobileCard)
          ) : (
            <div className="mobile-record-list__empty">暂无环境变量</div>
          )}
        </div>
      ) : (
        <Table
          columns={columns}
          data={envVars}
          loading={loading}
          pagination={{ pageSize: 10 }}
          scroll={{ x: 1200 }}
          rowKey="id"
          rowSelection={{
            type: 'checkbox',
            selectedRowKeys,
            onChange: (keys) => setSelectedRowKeys(keys as number[]),
          }}
        />
      )}

      <Modal
        title={editingEnv ? '编辑环境变量' : '新建环境变量'}
        visible={visible}
        onOk={handleSubmit}
        onCancel={() => setVisible(false)}
        autoFocus={false}
        style={{ width: '90%', maxWidth: 600 }}
      >
        <Form form={form} layout="vertical">
          <FormItem label="变量名" field="key" rules={[{ required: true, message: '请输入变量名' }]}>
            <Input placeholder="例如: API_KEY" />
          </FormItem>
          <FormItem label="变量值" field="value" rules={[{ required: true, message: '请输入变量值' }]}>
            <Input.TextArea placeholder="变量值" rows={3} />
          </FormItem>
          <FormItem label="描述" field="remark">
            <Input placeholder="变量描述" />
          </FormItem>
        </Form>
      </Modal>

      {/* 批量导入弹窗 */}
      <Modal
        title="批量导入环境变量"
        visible={batchVisible}
        onOk={handleBatchImport}
        confirmLoading={batchLoading}
        onCancel={() => {
          setBatchVisible(false);
          setBatchContent('');
          setAllowOverwrite(false);
        }}
        autoFocus={false}
        style={{ width: '90%', maxWidth: 600 }}
        okText="导入"
      >
        <Space direction="vertical" style={{ width: '100%' }} size="medium">
          <Input.TextArea
            value={batchContent}
            onChange={setBatchContent}
            placeholder={"# 支持标准 .env 格式，每行一个变量\nAPI_KEY=your_api_key\nDB_HOST=localhost\nDEBUG=true\n# 注释行会被忽略"}
            rows={10}
            style={{ fontFamily: 'monospace', fontSize: 13 }}
          />
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <Switch checked={allowOverwrite} onChange={setAllowOverwrite} />
            <span style={{ color: 'var(--color-text-2)' }}>允许覆盖同名变量</span>
            <span style={{ color: 'var(--color-text-3)', fontSize: 12 }}>
              （关闭时若存在同名变量则阻止导入）
            </span>
          </div>
        </Space>
      </Modal>
    </Card>
  );
};

export default Env;
