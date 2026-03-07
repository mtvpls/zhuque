import React, { useState, useEffect } from 'react';
import {
  Card,
  Button,
  Modal,
  Message,
  Space,
  Typography,
  Input,
  Alert,
  Spin,
} from '@arco-design/web-react';
import { authApi } from '@/api/auth';
import type { TotpSetupResponse } from '@/types/totp';

const { Title, Paragraph, Text } = Typography;

const TotpSettings: React.FC = () => {
  const [enabled, setEnabled] = useState(false);
  const [loading, setLoading] = useState(false);
  const [setupVisible, setSetupVisible] = useState(false);
  const [setupData, setSetupData] = useState<TotpSetupResponse | null>(null);
  const [verifyCode, setVerifyCode] = useState('');
  const [verifying, setVerifying] = useState(false);

  useEffect(() => {
    loadStatus();
  }, []);

  const loadStatus = async () => {
    try {
      const res = await authApi.getTotpStatus();
      setEnabled(res.enabled);
    } catch (error) {
      Message.error('获取TOTP状态失败');
    }
  };

  const handleSetup = async () => {
    setLoading(true);
    try {
      const res = await authApi.setupTotp();
      setSetupData(res);
      setSetupVisible(true);
    } catch (error) {
      Message.error('初始化TOTP失败');
    } finally {
      setLoading(false);
    }
  };

  const handleEnable = async () => {
    if (!setupData || !verifyCode) {
      Message.warning('请输入验证码');
      return;
    }

    setVerifying(true);
    try {
      await authApi.enableTotp({
        secret: setupData.secret,
        backup_codes: setupData.backup_codes,
        code: verifyCode,
      });
      Message.success('TOTP已启用');
      setEnabled(true);
      setSetupVisible(false);
      setSetupData(null);
      setVerifyCode('');
    } catch (error: any) {
      Message.error(error.response?.data || '启用失败，请检查验证码');
    } finally {
      setVerifying(false);
    }
  };

  const handleDisable = () => {
    let disableCode = '';
    Modal.confirm({
      title: '确认禁用TOTP',
      content: (
        <div>
          <p style={{ marginBottom: 16 }}>禁用后，登录将不再需要验证码。</p>
          <p style={{ marginBottom: 8, fontWeight: 'bold' }}>请输入TOTP验证码以确认：</p>
          <Input
            placeholder="请输入6位验证码"
            maxLength={6}
            onChange={(value) => {
              disableCode = value;
            }}
            autoFocus
          />
        </div>
      ),
      onOk: async () => {
        if (!disableCode || disableCode.length !== 6) {
          Message.error('请输入6位验证码');
          return Promise.reject();
        }
        try {
          await authApi.disableTotp(disableCode);
          Message.success('TOTP已禁用');
          setEnabled(false);
        } catch (error: any) {
          Message.error(error.response?.data || '禁用失败，请检查验证码');
          return Promise.reject();
        }
      },
    });
  };

  const handleRegenerateBackupCodes = () => {
    let regenerateCode = '';
    Modal.confirm({
      title: '重新生成备用码',
      content: (
        <div>
          <p style={{ marginBottom: 16 }}>重新生成后，旧的备用码将失效。</p>
          <p style={{ marginBottom: 8, fontWeight: 'bold' }}>请输入TOTP验证码以确认：</p>
          <Input
            placeholder="请输入6位验证码"
            maxLength={6}
            onChange={(value) => {
              regenerateCode = value;
            }}
            autoFocus
          />
        </div>
      ),
      onOk: async () => {
        if (!regenerateCode || regenerateCode.length !== 6) {
          Message.error('请输入6位验证码');
          return Promise.reject();
        }
        try {
          const res = await authApi.regenerateBackupCodes(regenerateCode);
          Modal.info({
            title: '新的备用恢复码',
            content: (
              <div>
                <Alert
                  type="warning"
                  content="请妥善保存这些备用码，每个备用码只能使用一次"
                  style={{ marginBottom: 16 }}
                />
                <div style={{ fontFamily: 'monospace', fontSize: 14 }}>
                  {res.backup_codes.map((code, index) => (
                    <div key={index} style={{ marginBottom: 8 }}>
                      {code}
                    </div>
                  ))}
                </div>
              </div>
            ),
          });
        } catch (error: any) {
          Message.error(error.response?.data || '重新生成失败，请检查验证码');
          return Promise.reject();
        }
      },
    });
  };

  return (
    <Card title="TOTP 二次验证" bordered={false}>
      <Space direction="vertical" size="large" style={{ width: '100%' }}>
        <div>
          <Paragraph>
            TOTP（Time-based One-Time Password）是一种基于时间的一次性密码验证方式，
            可以为您的账户提供额外的安全保护。
          </Paragraph>
          <Paragraph>
            状态：<Text bold>{enabled ? '已启用' : '未启用'}</Text>
          </Paragraph>
        </div>

        {!enabled ? (
          <Button type="primary" onClick={handleSetup} loading={loading}>
            启用 TOTP
          </Button>
        ) : (
          <Space>
            <Button onClick={handleDisable}>禁用 TOTP</Button>
            <Button onClick={handleRegenerateBackupCodes}>重新生成备用码</Button>
          </Space>
        )}
      </Space>

      <Modal
        title="设置 TOTP"
        visible={setupVisible}
        onCancel={() => {
          setSetupVisible(false);
          setSetupData(null);
          setVerifyCode('');
        }}
        footer={null}
        style={{ width: 600 }}
      >
        {setupData ? (
          <Space direction="vertical" size="large" style={{ width: '100%' }}>
            <Alert
              type="info"
              content="请使用验证器应用（如 Google Authenticator、Microsoft Authenticator）扫描二维码"
            />

            <div style={{ textAlign: 'center' }}>
              <img
                src={setupData.qr_code}
                alt="QR Code"
                style={{ maxWidth: '100%', height: 'auto' }}
              />
            </div>

            <div>
              <Paragraph>或手动输入密钥：</Paragraph>
              <Input
                value={setupData.secret}
                readOnly
                style={{ fontFamily: 'monospace' }}
              />
            </div>

            <div>
              <Title heading={6}>备用恢复码</Title>
              <Alert
                type="warning"
                content="请妥善保存这些备用码，当您无法使用验证器时可以使用它们登录。每个备用码只能使用一次。"
                style={{ marginBottom: 12 }}
              />
              <div
                style={{
                  fontFamily: 'monospace',
                  fontSize: 14,
                  backgroundColor: '#f7f8fa',
                  padding: 12,
                  borderRadius: 4,
                }}
              >
                {setupData.backup_codes.map((code, index) => (
                  <div key={index} style={{ marginBottom: 4 }}>
                    {code}
                  </div>
                ))}
              </div>
            </div>

            <div>
              <Paragraph>请输入验证器中的6位验证码以完成设置：</Paragraph>
              <Input
                placeholder="验证码"
                value={verifyCode}
                onChange={setVerifyCode}
                maxLength={6}
                style={{ marginBottom: 12 }}
              />
              <Button
                type="primary"
                long
                onClick={handleEnable}
                loading={verifying}
              >
                确认启用
              </Button>
            </div>
          </Space>
        ) : (
          <div style={{ textAlign: 'center', padding: 40 }}>
            <Spin />
          </div>
        )}
      </Modal>
    </Card>
  );
};

export default TotpSettings;
