import React, { useState } from 'react';
import { Outlet, useNavigate, useLocation } from 'react-router-dom';
import {
  Layout,
  Menu,
  Avatar,
  Dropdown,
  Space,
  Button,
} from '@arco-design/web-react';
import {
  IconDashboard,
  IconSchedule,
  IconFile,
  IconSettings,
  IconStorage,
  IconHistory,
  IconApps,
  IconSync,
  IconMenuFold,
  IconMenuUnfold,
  IconPoweroff,
  IconCode,
  IconNotification,
} from '@arco-design/web-react/icon';
import { useUserStore } from '@/stores/user';
import './BasicLayout.css';

const { Header, Sider, Content } = Layout;
const MenuItem = Menu.Item;

const BasicLayout: React.FC = () => {
  const [collapsed, setCollapsed] = useState(false);
  const navigate = useNavigate();
  const location = useLocation();
  const { logout } = useUserStore();

  const menuItems = [
    { key: '/', icon: <IconDashboard />, label: '仪表盘' },
    { key: '/tasks', icon: <IconSchedule />, label: '定时任务' },
    { key: '/scripts', icon: <IconFile />, label: '脚本管理' },
    { key: '/env', icon: <IconSettings />, label: '环境变量' },
    { key: '/dependences', icon: <IconApps />, label: '依赖管理' },
    { key: '/subscriptions', icon: <IconSync />, label: '订阅管理' },
    { key: '/logs', icon: <IconHistory />, label: '执行日志' },
    { key: '/terminal', icon: <IconCode />, label: '终端' },
    { key: '/remote', icon: <IconStorage />, label: '远程机器' },
    { key: '/notifications', icon: <IconNotification />, label: '通知管理' },
    { key: '/config', icon: <IconStorage />, label: '系统配置' },
  ];

  const handleLogout = () => {
    logout();
    navigate('/login');
  };

  const droplist = (
    <Menu>
      <MenuItem key="logout" onClick={handleLogout}>
        <Space>
          <IconPoweroff />
          退出登录
        </Space>
      </MenuItem>
    </Menu>
  );

  return (
    <Layout className="basic-layout">
      <Sider
        collapsed={collapsed}
        collapsible
        trigger={null}
        breakpoint="lg"
        onCollapse={setCollapsed}
        className="layout-sider"
      >
        <div className="logo">
          <h1>{collapsed ? '朱' : '朱雀'}</h1>
        </div>
        <Menu
          selectedKeys={[location.pathname]}
          onClickMenuItem={(key) => navigate(key)}
          style={{ width: '100%' }}
        >
          {menuItems.map((item) => (
            <MenuItem key={item.key}>
              {item.icon}
              {item.label}
            </MenuItem>
          ))}
        </Menu>
      </Sider>
      <Layout>
        <Header className="layout-header">
          <Button
            shape="circle"
            icon={collapsed ? <IconMenuUnfold /> : <IconMenuFold />}
            onClick={() => setCollapsed(!collapsed)}
          />
          <Dropdown droplist={droplist} position="br" trigger="click">
            <Avatar size={32} style={{ cursor: 'pointer' }}>
              Admin
            </Avatar>
          </Dropdown>
        </Header>
        <Content className="layout-content">
          <Outlet />
        </Content>
      </Layout>
    </Layout>
  );
};

export default BasicLayout;
