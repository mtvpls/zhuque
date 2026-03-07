import React, { useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { Form, Input, Button, Card, Message } from '@arco-design/web-react';
import { IconLock, IconUser } from '@arco-design/web-react/icon';
import { authApi } from '@/api/auth';

const FormItem = Form.Item;

const InitialSetup: React.FC = () => {
  const [loading, setLoading] = useState(false);
  const navigate = useNavigate();
  const [form] = Form.useForm();

  const handleSubmit = async (values: any) => {
    if (values.password !== values.confirmPassword) {
      Message.error('两次密码不一致');
      return;
    }

    setLoading(true);
    try {
      await authApi.initialSetup(values.username, values.password);
      Message.success('账号设置成功，请登录');
      navigate('/login');
    } catch (error: any) {
      Message.error(error.response?.data || '设置失败');
    } finally {
      setLoading(false);
    }
  };

  return (
    <div style={{
      display: 'flex',
      justifyContent: 'center',
      alignItems: 'center',
      minHeight: '100vh',
      background: '#f0f2f5'
    }}>
      <Card
        style={{ width: 400 }}
        bordered={false}
      >
        <div style={{ textAlign: 'center', marginBottom: 24 }}>
          <h1>朱雀 - 初始设置</h1>
          <p style={{ color: '#86909c' }}>请设置管理员账号和密码</p>
        </div>

        <Form
          form={form}
          onSubmit={handleSubmit}
          autoComplete="off"
          layout="vertical"
        >
          <FormItem
            field="username"
            rules={[
              { required: true, message: '请输入用户名' },
              { minLength: 3, message: '用户名至少3个字符' }
            ]}
          >
            <Input
              prefix={<IconUser />}
              placeholder="用户名"
              size="large"
            />
          </FormItem>

          <FormItem
            field="password"
            rules={[
              { required: true, message: '请输入密码' },
              { minLength: 6, message: '密码至少6个字符' }
            ]}
          >
            <Input.Password
              prefix={<IconLock />}
              placeholder="密码"
              size="large"
            />
          </FormItem>

          <FormItem
            field="confirmPassword"
            rules={[
              { required: true, message: '请确认密码' }
            ]}
          >
            <Input.Password
              prefix={<IconLock />}
              placeholder="确认密码"
              size="large"
            />
          </FormItem>

          <FormItem>
            <Button
              type="primary"
              htmlType="submit"
              long
              size="large"
              loading={loading}
            >
              完成设置
            </Button>
          </FormItem>
        </Form>
      </Card>
    </div>
  );
};

export default InitialSetup;
