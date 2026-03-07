import React, { useState, useEffect } from 'react';
import { useNavigate } from 'react-router-dom';
import { Form, Input, Button, Card, Message } from '@arco-design/web-react';
import { IconLock, IconUser } from '@arco-design/web-react/icon';
import { authApi } from '@/api/auth';
import { useUserStore } from '@/stores/user';
import './Login.css';

const FormItem = Form.Item;

type LoginStep = 'credentials' | 'totp';

const Login: React.FC = () => {
  const [loading, setLoading] = useState(false);
  const [step, setStep] = useState<LoginStep>('credentials');
  const [sessionToken, setSessionToken] = useState<string>('');
  const navigate = useNavigate();
  const { setToken } = useUserStore();
  const [form] = Form.useForm();
  const [totpForm] = Form.useForm();

  // 检查是否需要初始设置
  useEffect(() => {
    authApi.checkInitialSetup()
      .then(res => {
        if (res.needs_setup) {
          navigate('/setup');
        }
      })
      .catch(() => {
        // 忽略错误，继续显示登录页面
      });
  }, [navigate]);

  const handleCredentialsSubmit = async (values: any) => {
    setLoading(true);
    try {
      const res = await authApi.login(values.username, values.password);

      if (res.requires_totp && res.session_token) {
        // 需要TOTP验证
        setSessionToken(res.session_token);
        setStep('totp');
        Message.info('请输入验证码');
      } else if (res.token) {
        // 直接登录成功
        setToken(res.token);
        Message.success('登录成功');
        navigate('/');
      }
    } catch (error: any) {
      Message.error(error.response?.data || '登录失败');
    } finally {
      setLoading(false);
    }
  };

  const handleTotpSubmit = async (values: any) => {
    setLoading(true);
    try {
      const res = await authApi.verifyTotp({
        session_token: sessionToken,
        code: values.code,
      });
      setToken(res.token);
      Message.success('登录成功');
      navigate('/');
    } catch (error: any) {
      Message.error(error.response?.data || '验证码错误');
    } finally {
      setLoading(false);
    }
  };

  const handleBackToCredentials = () => {
    setStep('credentials');
    setSessionToken('');
    totpForm.resetFields();
  };

  return (
    <div className="login-container">
      <Card className="login-card" bordered={false}>
        <div className="login-header">
          <h1>朱雀</h1>
          <p>轻量级定时任务管理平台</p>
        </div>

        {step === 'credentials' ? (
          <Form
            form={form}
            onSubmit={handleCredentialsSubmit}
            autoComplete="off"
            layout="vertical"
            style={{ width: '100%' }}
          >
            <FormItem field="username" rules={[{ required: true, message: '请输入用户名' }]}>
              <Input
                prefix={<IconUser />}
                placeholder="用户名"
                size="large"
                style={{ width: '100%' }}
              />
            </FormItem>
            <FormItem field="password" rules={[{ required: true, message: '请输入密码' }]}>
              <Input.Password
                prefix={<IconLock />}
                placeholder="密码"
                size="large"
                style={{ width: '100%' }}
              />
            </FormItem>
            <FormItem>
              <Button
                type="primary"
                htmlType="submit"
                long
                size="large"
                loading={loading}
                style={{ width: '100%' }}
              >
                登录
              </Button>
            </FormItem>
          </Form>
        ) : (
          <Form
            form={totpForm}
            onSubmit={handleTotpSubmit}
            autoComplete="off"
            layout="vertical"
            style={{ width: '100%' }}
          >
            <FormItem
              field="code"
              rules={[{ required: true, message: '请输入验证码' }]}
              extra="请输入验证器中的6位验证码，或使用16位备用恢复码"
            >
              <Input
                prefix={<IconLock />}
                placeholder="验证码"
                size="large"
                style={{ width: '100%' }}
                maxLength={16}
              />
            </FormItem>
            <FormItem>
              <Button
                type="primary"
                htmlType="submit"
                long
                size="large"
                loading={loading}
                style={{ width: '100%', marginBottom: 12 }}
              >
                验证
              </Button>
              <Button
                type="text"
                long
                size="large"
                onClick={handleBackToCredentials}
                style={{ width: '100%' }}
              >
                返回重新登录
              </Button>
            </FormItem>
          </Form>
        )}
      </Card>
    </div>
  );
};

export default Login;
