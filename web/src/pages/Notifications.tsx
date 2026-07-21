import React, { useEffect, useState } from 'react';
import {
  Card,
  Form,
  Input,
  Select,
  Switch,
  Button,
  Space,
  Message,
  Modal,
  Divider,
  Typography,
  InputNumber,
  Spin,
  Tag,
  Popconfirm,
  Empty,
} from '@arco-design/web-react';
import {
  IconPlus,
  IconEdit,
  IconDelete,
  IconSend,
  IconSave,
} from '@arco-design/web-react/icon';
import { notificationApi } from '@/api/notification';
import type { NotificationConfig, ChannelConfig } from '@/types';

const { Text } = Typography;
const FormItem = Form.Item;
const Option = Select.Option;

// ─── 响应式 ───────────────────────────────────────────────────────────────────

const useMobile = () => {
  const [mobile, setMobile] = useState(() => window.innerWidth < 640);
  useEffect(() => {
    const h = () => setMobile(window.innerWidth < 640);
    window.addEventListener('resize', h);
    return () => window.removeEventListener('resize', h);
  }, []);
  return mobile;
};

// ─── 渠道类型元信息 ────────────────────────────────────────────────────────────

const CHANNEL_TYPES = [
  { value: 'telegram',  label: 'Telegram',      color: 'blue'     },
  { value: 'pushplus',  label: 'PushPlus',      color: 'green'    },
  { value: 'wecom',     label: '企业微信',       color: 'arcoblue' },
  { value: 'smtp',      label: 'SMTP 邮件',     color: 'orange'   },
  { value: 'resend',    label: 'Resend',        color: 'purple'   },
  { value: 'webhook',   label: 'Webhook',       color: 'gray'     },
  { value: 'dingtalk',  label: '钉钉',          color: 'orangered'},
  { value: 'feishu',    label: '飞书',          color: 'cyan'     },
  { value: 'bark',      label: 'Bark',          color: 'lime'     },
  { value: 'ntfy',      label: 'ntfy',          color: 'pinkpurple'},
];

const typeLabel = (t: string) => CHANNEL_TYPES.find(c => c.value === t)?.label ?? t;
const typeColor = (t: string) => CHANNEL_TYPES.find(c => c.value === t)?.color ?? 'gray';
const createChannelId = () => globalThis.crypto?.randomUUID?.()
  ?? `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;

// ─── 各渠道默认配置 ────────────────────────────────────────────────────────────

const defaults: Record<string, object> = {
  telegram: { bot_token: '', chat_id: '', proxy: '' },
  pushplus: { token: '', topic: '' },
  wecom:    { key: '' },
  smtp:     { host: '', port: 465, username: '', password: '', from: '', to: [], use_tls: true },
  resend:   { api_key: '', from: '', to: [] },
  webhook:  { url: '', method: 'POST', headers: {}, body_template: '' },
  dingtalk: { access_token: '', secret: '' },
  feishu:   { webhook_url: '', sign_key: '' },
  bark:     { server_url: '', device_key: '', sound: '', group: '' },
  ntfy:     { server_url: '', topic: '', token: '', priority: 0 }
};

// ─── 渠道配置表单（按类型渲染字段）────────────────────────────────────────────

interface ConfigFormProps {
  type: string;
  value: Record<string, any>;
  onChange: (v: Record<string, any>) => void;
  isMobile: boolean;
}

type HeaderRow = { id: number; key: string; value: string };

const ConfigForm: React.FC<ConfigFormProps> = ({ type, value, onChange, isMobile }) => {
  const layout  = isMobile ? 'vertical' : 'horizontal';
  const lc      = isMobile ? undefined : { span: 7 };
  const wc      = isMobile ? undefined : { span: 17 };
  const set     = (k: string, v: any) => onChange({ ...value, [k]: v });

  // webhook headers 本地状态
  const [headerRows, setHeaderRows] = useState<HeaderRow[]>(() => {
    const h = value.headers ?? {};
    return Object.entries(h).map(([k, v], i) => ({ id: i, key: k, value: String(v) }));
  });

  const syncHeaders = (rows: HeaderRow[]) => {
    const h: Record<string, string> = {};
    rows.filter(r => r.key).forEach(r => { h[r.key] = r.value; });
    onChange({ ...value, headers: h });
  };

  const addHeader = () => {
    const rows = [...headerRows, { id: Date.now(), key: '', value: '' }];
    setHeaderRows(rows);
  };

  const removeHeader = (id: number) => {
    const rows = headerRows.filter(r => r.id !== id);
    setHeaderRows(rows);
    syncHeaders(rows);
  };

  const updateHeader = (id: number, field: 'key' | 'value', v: string) => {
    const rows = headerRows.map(r => r.id === id ? { ...r, [field]: v } : r);
    setHeaderRows(rows);
    syncHeaders(rows);
  };

  if (type === 'telegram') return (
    <Form layout={layout} labelCol={lc} wrapperCol={wc}>
      <FormItem label="Bot Token" required>
        <Input placeholder="110201543:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw"
          value={value.bot_token} onChange={v => set('bot_token', v)} />
      </FormItem>
      <FormItem label="Chat ID" required>
        <Input placeholder="-1001234567890 或个人数字 ID"
          value={value.chat_id} onChange={v => set('chat_id', v)} />
      </FormItem>
      <FormItem label="代理地址">
        <Input placeholder="http://127.0.0.1:7890（可选）"
          value={value.proxy} onChange={v => set('proxy', v)} />
      </FormItem>
    </Form>
  );

  if (type === 'pushplus') return (
    <Form layout={layout} labelCol={lc} wrapperCol={wc}>
      <FormItem label="Token" required>
        <Input placeholder="PushPlus 用户 Token"
          value={value.token} onChange={v => set('token', v)} />
      </FormItem>
      <FormItem label="群组编码">
        <Input placeholder="不填则发给自己（可选）"
          value={value.topic} onChange={v => set('topic', v)} />
      </FormItem>
    </Form>
  );

  if (type === 'wecom') return (
    <Form layout={layout} labelCol={lc} wrapperCol={wc}>
      <FormItem label="Key" required>
        <Input placeholder="企业微信机器人 key（webhook URL 中 key= 后面的部分）"
          value={value.key} onChange={v => set('key', v)} />
      </FormItem>
    </Form>
  );

  if (type === 'dingtalk') return (
    <Form layout={layout} labelCol={lc} wrapperCol={wc}>
      <FormItem label="Access Token" required>
        <Input placeholder="钉钉机器人 access_token"
          value={value.access_token} onChange={v => set('access_token', v)} />
      </FormItem>
      <FormItem label="签名密钥">
        <Input placeholder="可选，开启安全设置后填写"
          value={value.secret} onChange={v => set('secret', v)} />
      </FormItem>
    </Form>
  );

  if (type === 'feishu') return (
    <Form layout={layout} labelCol={lc} wrapperCol={wc}>
      <FormItem label="Webhook URL" required>
        <Input placeholder="https://open.feishu.cn/open-apis/bot/v2/hook/..."
          value={value.webhook_url} onChange={v => set('webhook_url', v)} />
      </FormItem>
      <FormItem label="签名 Key">
        <Input placeholder="可选，开启签名校验后填写"
          value={value.sign_key} onChange={v => set('sign_key', v)} />
      </FormItem>
    </Form>
  );

  if (type === 'bark') return (
    <Form layout={layout} labelCol={lc} wrapperCol={wc}>
      <FormItem label="服务地址" required>
        <Input placeholder="https://api.day.app"
          value={value.server_url} onChange={v => set('server_url', v)} />
      </FormItem>
      <FormItem label="Device Key" required>
        <Input placeholder="你的 Bark Device Key"
          value={value.device_key} onChange={v => set('device_key', v)} />
      </FormItem>
      <FormItem label="铃声">
        <Input placeholder="可选，如 minuet"
          value={value.sound} onChange={v => set('sound', v)} />
      </FormItem>
      <FormItem label="分组">
        <Input placeholder="可选，消息分组名称"
          value={value.group} onChange={v => set('group', v)} />
      </FormItem>
    </Form>
  );

  if (type === 'ntfy') return (
    <Form layout={layout} labelCol={lc} wrapperCol={wc}>
      <FormItem label="服务地址" required>
        <Input placeholder="https://ntfy.sh"
          value={value.server_url} onChange={v => set('server_url', v)} />
      </FormItem>
      <FormItem label="Topic" required>
        <Input placeholder="你的 topic 名称"
          value={value.topic} onChange={v => set('topic', v)} />
      </FormItem>
      <FormItem label="Token">
        <Input placeholder="可选，私有服务器鉴权 token"
          value={value.token} onChange={v => set('token', v)} />
      </FormItem>
      <FormItem label="优先级">
        <InputNumber min={0} max={5} placeholder="0=默认 1=最低 5=最高"
          value={value.priority} onChange={v => set('priority', v ?? 0)} />
      </FormItem>
    </Form>
  );

  if (type === 'smtp') return (
    <Form layout={layout} labelCol={lc} wrapperCol={wc}>
      {isMobile ? (
        <>
          <FormItem label="SMTP 服务器" required>
            <Input placeholder="smtp.gmail.com" value={value.host} onChange={v => set('host', v)} />
          </FormItem>
          <FormItem label="端口">
            <InputNumber min={1} max={65535} value={value.port} onChange={v => set('port', v)} style={{ width: '100%' }} />
          </FormItem>
        </>
      ) : (
        <FormItem label="服务器 / 端口" required>
          <div style={{ display: 'flex', gap: 8 }}>
            <Input placeholder="smtp.gmail.com" value={value.host} onChange={v => set('host', v)} style={{ flex: 1 }} />
            <InputNumber min={1} max={65535} value={value.port} onChange={v => set('port', v)} style={{ width: 90 }} />
          </div>
        </FormItem>
      )}
      <FormItem label="用户名" required>
        <Input value={value.username} onChange={v => set('username', v)} />
      </FormItem>
      <FormItem label="密码" required>
        <Input.Password value={value.password} onChange={v => set('password', v)} />
      </FormItem>
      <FormItem label="发件人" required>
        <Input placeholder="no-reply@example.com" value={value.from} onChange={v => set('from', v)} />
      </FormItem>
      <FormItem label="收件人" required>
        <Input placeholder="多个地址用英文逗号分隔"
          value={Array.isArray(value.to) ? value.to.join(', ') : value.to}
          onChange={v => set('to', v.split(',').map((s: string) => s.trim()).filter(Boolean))} />
      </FormItem>
      <FormItem label="SSL/TLS">
        <Switch checked={value.use_tls} onChange={v => set('use_tls', v)} />
      </FormItem>
    </Form>
  );

  if (type === 'resend') return (
    <Form layout={layout} labelCol={lc} wrapperCol={wc}>
      <FormItem label="API Key" required>
        <Input.Password placeholder="re_xxxxxxxx" value={value.api_key} onChange={v => set('api_key', v)} />
      </FormItem>
      <FormItem label="发件人" required>
        <Input placeholder="Zhuque <no-reply@example.com>" value={value.from} onChange={v => set('from', v)} />
      </FormItem>
      <FormItem label="收件人" required>
        <Input placeholder="多个地址用英文逗号分隔"
          value={Array.isArray(value.to) ? value.to.join(', ') : value.to}
          onChange={v => set('to', v.split(',').map((s: string) => s.trim()).filter(Boolean))} />
      </FormItem>
    </Form>
  );

  if (type === 'webhook') return (
    <Form layout={layout} labelCol={lc} wrapperCol={wc}>
      {isMobile ? (
        <>
          <FormItem label="请求方式">
            <Select value={value.method} onChange={v => set('method', v)}>
              {['GET','POST','PUT','PATCH'].map(m => <Option key={m} value={m}>{m}</Option>)}
            </Select>
          </FormItem>
          <FormItem label="URL" required>
            <Input placeholder="https://example.com/hook" value={value.url} onChange={v => set('url', v)} />
          </FormItem>
        </>
      ) : (
        <FormItem label="方式 / URL" required>
          <div style={{ display: 'flex', gap: 8 }}>
            <Select value={value.method} onChange={v => set('method', v)} style={{ width: 100 }}>
              {['GET','POST','PUT','PATCH'].map(m => <Option key={m} value={m}>{m}</Option>)}
            </Select>
            <Input placeholder="https://example.com/hook" value={value.url} onChange={v => set('url', v)} style={{ flex: 1 }} />
          </div>
        </FormItem>
      )}
      <FormItem label="自定义 Headers">
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
          {headerRows.map(row => (
            <div key={row.id} style={{ display: 'flex', gap: 6 }}>
              <Input placeholder="Header 名" value={row.key}
                onChange={v => updateHeader(row.id, 'key', v)} style={{ flex: 1 }} />
              <Input placeholder="值" value={row.value}
                onChange={v => updateHeader(row.id, 'value', v)} style={{ flex: 2 }} />
              <Button size="small" type="text" icon={<IconDelete />}
                onClick={() => removeHeader(row.id)} />
            </div>
          ))}
          <Button size="small" type="dashed" icon={<IconPlus />} onClick={addHeader}>
            添加 Header
          </Button>
        </div>
      </FormItem>
      {value.method !== 'GET' && (
        <FormItem label="Body 模板">
          <Input.TextArea
            placeholder={'留空则使用默认 JSON：\n{"title":"{title}","content":"{content}"}\n\n支持变量：{title} {content}'}
            value={value.body_template}
            onChange={v => set('body_template', v)}
            autoSize={{ minRows: 3, maxRows: 8 }}
          />
        </FormItem>
      )}
    </Form>
  );

  return null;
};

// ─── 主页面 ───────────────────────────────────────────────────────────────────

const Notifications: React.FC = () => {
  const isMobile = useMobile();

  const [loading,  setLoading]  = useState(false);
  const [saving,   setSaving]   = useState(false);
  const [testing,  setTesting]  = useState<string | null>(null);

  // 全局开关
  const [enabled,   setEnabled]   = useState(false);
  const [onSuccess, setOnSuccess] = useState(false);
  const [onFailure, setOnFailure] = useState(true);
  const [onKilled,  setOnKilled]  = useState(true);
  const [onLogin,   setOnLogin]   = useState(false);

  // 渠道列表
  const [channels, setChannels] = useState<ChannelConfig[]>([]);

  // 弹窗状态
  const [modalVisible, setModalVisible] = useState(false);
  const [editingId,    setEditingId]    = useState<string | null>(null);
  const [modalType,    setModalType]    = useState('telegram');
  const [modalName,    setModalName]    = useState('');
  const [modalEnabled, setModalEnabled] = useState(true);
  const [modalConfig,  setModalConfig]  = useState<Record<string, any>>({});

  // ─── 加载 ──────────────────────────────────────────────────────────────────

  useEffect(() => {
    setLoading(true);
    notificationApi.getConfig()
      .then(cfg => {
        setEnabled(cfg.enabled);
        setOnSuccess(cfg.on_success);
        setOnFailure(cfg.on_failure);
        setOnKilled(cfg.on_killed);
        setOnLogin(cfg.on_login ?? false);
        setChannels(cfg.channels ?? []);
      })
      .catch(() => Message.error('加载通知配置失败'))
      .finally(() => setLoading(false));
  }, []);

  // ─── 弹窗操作 ──────────────────────────────────────────────────────────────

  const openAdd = () => {
    setEditingId(null);
    setModalType('telegram');
    setModalName('');
    setModalEnabled(true);
    setModalConfig({ ...(defaults['telegram'] as object) });
    setModalVisible(true);
  };

  const openEdit = (ch: ChannelConfig) => {
    setEditingId(ch.id);
    setModalType(ch.type);
    setModalName(ch.name);
    setModalEnabled(ch.enabled);
    setModalConfig({ ...(ch.config as Record<string, any>) });
    setModalVisible(true);
  };

  const handleModalTypeChange = (t: string) => {
    setModalType(t);
    setModalConfig({ ...(defaults[t] as object) });
  };

  const handleModalOk = () => {
    if (!modalName.trim()) { Message.warning('请填写渠道名称'); return; }
    const entry: ChannelConfig = {
      id:      editingId ?? createChannelId(),
      name:    modalName.trim(),
      type:    modalType,
      enabled: modalEnabled,
      config:  modalConfig as any,
    };
    if (editingId) {
      setChannels(prev => prev.map(c => c.id === editingId ? entry : c));
    } else {
      setChannels(prev => [...prev, entry]);
    }
    setModalVisible(false);
  };

  const handleDelete = (id: string) => {
    setChannels(prev => prev.filter(c => c.id !== id));
  };

  const handleToggle = (id: string, val: boolean) => {
    setChannels(prev => prev.map(c => c.id === id ? { ...c, enabled: val } : c));
  };

  // ─── 测试 ──────────────────────────────────────────────────────────────────

  const handleTest = async (ch: ChannelConfig) => {
    setTesting(ch.id);
    try {
      await notificationApi.testChannel({ channel_type: ch.type, config: ch.config as any });
      Message.success(`${ch.name} 测试成功`);
    } catch (e: any) {
      Message.error(e.response?.data || '测试失败');
    } finally {
      setTesting(null);
    }
  };

  // ─── 保存 ──────────────────────────────────────────────────────────────────

  const handleSave = async () => {
    setSaving(true);
    try {
      const config: NotificationConfig = {
        enabled, on_success: onSuccess, on_failure: onFailure, on_killed: onKilled, on_login: onLogin,
        channels,
      };
      await notificationApi.updateConfig(config);
      Message.success('保存成功');
    } catch (e: any) {
      Message.error(e.response?.data || '保存失败');
    } finally {
      setSaving(false);
    }
  };

  // ─── 渲染 ──────────────────────────────────────────────────────────────────

  return (
    <Spin loading={loading} style={{ width: '100%' }}>
      <div style={{ maxWidth: 820, margin: '0 auto', padding: isMobile ? '0 8px 32px' : '0 16px 32px' }}>

        {/* ── 页面标题 + 保存按钮 ── */}
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 16 }}>
          <Typography.Title heading={5} style={{ margin: 0 }}>通知管理</Typography.Title>
          <Button type="primary" icon={<IconSave />} loading={saving}
            onClick={handleSave} size="small">
            保存配置
          </Button>
        </div>

        {/* ── 全局开关 ── */}
        <Card title="通知设置" style={{ marginBottom: 16 }}>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
            {/* 第一行：总开关 */}
            <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
              <Text>通知总开关</Text>
              <Switch checked={enabled} onChange={setEnabled} />
            </div>
            {/* 第二行：触发条件 */}
            <div>
              <Text style={{ fontSize: 13, color: 'var(--color-text-3)', display: 'block', marginBottom: 8 }}>
                触发条件
              </Text>
              <Space wrap size={[16, 8]}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                  <Switch size="small" checked={onSuccess} onChange={setOnSuccess} />
                  <Text style={{ fontSize: 13 }}>成功时</Text>
                </div>
                <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                  <Switch size="small" checked={onFailure} onChange={setOnFailure} />
                  <Text style={{ fontSize: 13 }}>失败时</Text>
                </div>
                <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                  <Switch size="small" checked={onKilled} onChange={setOnKilled} />
                  <Text style={{ fontSize: 13 }}>终止时</Text>
                </div>
                <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                  <Switch size="small" checked={onLogin} onChange={setOnLogin} />
                  <Text style={{ fontSize: 13 }}>登录时</Text>
                </div>
              </Space>
            </div>
          </div>
        </Card>

        {/* ── 渠道列表 ── */}
        <Card
          title="通知渠道"
          extra={
            <Button type="primary" size="small" icon={<IconPlus />} onClick={openAdd}>
              添加渠道
            </Button>
          }
          style={{ marginBottom: 16 }}
        >
          {channels.length === 0 ? (
            <Empty description="暂无通知渠道，点击右上角添加" />
          ) : (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
              {channels.map(ch => (
                <div key={ch.id} style={{
                  display: 'flex',
                  flexDirection: isMobile ? 'column' : 'row',
                  alignItems: isMobile ? 'stretch' : 'center',
                  gap: isMobile ? 6 : 8,
                  padding: '10px 12px', borderRadius: 6,
                  background: 'var(--color-fill-2)',
                }}>
                  {/* 名称 + 类型 tag + 状态 tag */}
                  <div style={{ display: 'flex', alignItems: 'center', gap: 8, flex: 1, minWidth: 0 }}>
                    <Text style={{ fontWeight: 500, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                      {ch.name}
                    </Text>
                    <Tag color={typeColor(ch.type)} size="small">{typeLabel(ch.type)}</Tag>
                    {!ch.enabled && <Tag color="gray" size="small">已禁用</Tag>}
                  </div>
                  {/* 开关 + 操作按钮 */}
                  <div style={{ display: 'flex', alignItems: 'center', gap: 4, justifyContent: isMobile ? 'flex-end' : 'flex-start' }}>
                    <Switch size="small" checked={ch.enabled}
                      onChange={v => handleToggle(ch.id, v)} />
                    <Button size="mini" type="text" icon={<IconSend />}
                      loading={testing === ch.id}
                      onClick={() => handleTest(ch)}>
                      {!isMobile && '测试'}
                    </Button>
                    <Button size="mini" type="text" icon={<IconEdit />}
                      onClick={() => openEdit(ch)}>
                      {!isMobile && '编辑'}
                    </Button>
                    <Popconfirm title="确认删除此渠道？" onOk={() => handleDelete(ch.id)}>
                      <Button size="mini" type="text" status="danger" icon={<IconDelete />}>
                        {!isMobile && '删除'}
                      </Button>
                    </Popconfirm>
                  </div>
                </div>
              ))}
            </div>
          )}
        </Card>

        {/* ── 脚本调用说明 ── */}
        <Card title="脚本内发送通知" style={{ marginBottom: 16 }}>
          <Text style={{ fontSize: 13, color: 'var(--color-text-3)' }}>
            任务执行时，以下调用方式可主动推送通知到已启用的所有渠道：
          </Text>
          <Divider orientation="left" style={{ fontSize: 12, color: '#999', margin: '12px 0 8px' }}>Shell</Divider>
          <pre style={{ background: 'var(--color-fill-2)', padding: '8px 12px', borderRadius: 4, fontSize: 12, margin: 0, overflowX: 'auto', whiteSpace: 'pre' }}>
            {`notify "签到结果" "账号A: +50豆\\n账号B: 失败"`}
          </pre>
          <Divider orientation="left" style={{ fontSize: 12, color: '#999', margin: '12px 0 8px' }}>Python</Divider>
          <pre style={{ background: 'var(--color-fill-2)', padding: '8px 12px', borderRadius: 4, fontSize: 12, margin: 0, overflowX: 'auto', whiteSpace: 'pre' }}>
            {`from notify import send\nsend("签到结果", "账号A: +50豆")`}
          </pre>
          <Divider orientation="left" style={{ fontSize: 12, color: '#999', margin: '12px 0 8px' }}>Node.js</Divider>
          <pre style={{ background: 'var(--color-fill-2)', padding: '8px 12px', borderRadius: 4, fontSize: 12, margin: 0, overflowX: 'auto', whiteSpace: 'pre' }}>
            {`const { sendNotify } = require('./sendNotify')\nsendNotify('签到结果', '账号A: +50豆')`}
          </pre>
        </Card>

      </div>

      {/* ── 添加 / 编辑 弹窗 ── */}
      <Modal
        title={editingId ? '编辑通知渠道' : '添加通知渠道'}
        visible={modalVisible}
        onOk={handleModalOk}
        onCancel={() => setModalVisible(false)}
        style={{ width: isMobile ? '95vw' : 560 }}
        unmountOnExit
      >
        <Form layout="vertical">
          <FormItem label="渠道名称" required>
            <Input placeholder="例：主TG机器人、备用Webhook"
              value={modalName} onChange={setModalName} />
          </FormItem>
          <FormItem label="渠道类型">
            <Select value={modalType} onChange={handleModalTypeChange} disabled={!!editingId}>
              {CHANNEL_TYPES.map(t => (
                <Option key={t.value} value={t.value}>{t.label}</Option>
              ))}
            </Select>
          </FormItem>
          <FormItem label="启用">
            <Switch checked={modalEnabled} onChange={setModalEnabled} />
          </FormItem>
          <Divider style={{ margin: '8px 0' }} />
          <ConfigForm
            type={modalType}
            value={modalConfig}
            onChange={setModalConfig}
            isMobile={isMobile}
          />
        </Form>
      </Modal>
    </Spin>
  );
};

export default Notifications;
