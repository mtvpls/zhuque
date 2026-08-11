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
  IconStorage,
  IconMenuFold,
  IconMenuUnfold,
  IconPoweroff,
  IconNotification,
} from '@arco-design/web-react/icon';
import { useUserStore } from '@/stores/user';
import './BasicLayout.css';

const { Header, Sider, Content } = Layout;
const MenuItem = Menu.Item;

const SquareTerminalIcon: React.FC<React.SVGProps<SVGSVGElement>> = (props) => (
  <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="arco-icon" {...props}>
    <path d="m7 11 2-2-2-2" />
    <path d="M11 13h4" />
    <rect width="18" height="18" x="3" y="3" rx="2" />
  </svg>
);

const FileClockIcon: React.FC<React.SVGProps<SVGSVGElement>> = (props) => (
  <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="arco-icon" {...props}>
    <path d="M16 22h2a2 2 0 0 0 2-2V8a2.4 2.4 0 0 0-.706-1.706l-3.588-3.588A2.4 2.4 0 0 0 14 2H6a2 2 0 0 0-2 2v2.85" />
    <path d="M14 2v5a1 1 0 0 0 1 1h5" />
    <path d="M8 14v2.2l1.6 1" />
    <circle cx="8" cy="16" r="6" />
  </svg>
);

const CalendarSyncIcon: React.FC<React.SVGProps<SVGSVGElement>> = (props) => (
  <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="arco-icon" {...props}>
    <path d="M11 10v4h4" />
    <path d="m11 14 1.535-1.605a5 5 0 0 1 8 1.5" />
    <path d="M16 2v3" />
    <path d="m21 18-1.535 1.605a5 5 0 0 1-8-1.5" />
    <path d="M21 22v-4h-4" />
    <path d="M21 8.517V5a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h3.517" />
    <path d="M3 9h4" />
    <path d="M8 2v3" />
  </svg>
);

const KeyIcon: React.FC<React.SVGProps<SVGSVGElement>> = (props) => (
  <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="arco-icon" {...props}>
    <path d="m15.5 7.5 2.3 2.3a1 1 0 0 0 1.4 0l2.1-2.1a1 1 0 0 0 0-1.4L19 4" />
    <path d="m21 2-9.6 9.6" />
    <circle cx="7.5" cy="15.5" r="5.5" />
  </svg>
);

const FilePenIcon: React.FC<React.SVGProps<SVGSVGElement>> = (props) => (
  <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="arco-icon" {...props}>
    <path d="M12.659 22H18a2 2 0 0 0 2-2V8a2.4 2.4 0 0 0-.706-1.706l-3.588-3.588A2.4 2.4 0 0 0 14 2H6a2 2 0 0 0-2 2v9.34" />
    <path d="M14 2v5a1 1 0 0 0 1 1h5" />
    <path d="M10.378 12.622a1 1 0 0 1 3 3.003L8.36 20.637a2 2 0 0 1-.854.506l-2.867.837a.5.5 0 0 1-.62-.62l.836-2.869a2 2 0 0 1 .506-.853z" />
  </svg>
);

const PackageOpenIcon: React.FC<React.SVGProps<SVGSVGElement>> = (props) => (
  <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="arco-icon" {...props}>
    <path d="M12 22v-9" />
    <path d="M15.17 2.21a1.67 1.67 0 0 1 1.63 0L21 4.57a1.93 1.93 0 0 1 0 3.36L8.82 14.79a1.655 1.655 0 0 1-1.64 0L3 12.43a1.93 1.93 0 0 1 0-3.36z" />
    <path d="M20 13v3.87a2.06 2.06 0 0 1-1.11 1.83l-6 3.08a1.93 1.93 0 0 1-1.78 0l-6-3.08A2.06 2.06 0 0 1 4 16.87V13" />
    <path d="M21 12.43a1.93 1.93 0 0 0 0-3.36L8.83 2.2a1.64 1.64 0 0 0-1.63 0L3 4.57a1.93 1.93 0 0 0 0 3.36l12.18 6.86a1.636 1.636 0 0 0 1.63 0z" />
  </svg>
);

const BasicLayout: React.FC = () => {
  const [collapsed, setCollapsed] = useState(false);
  const navigate = useNavigate();
  const location = useLocation();
  const { logout } = useUserStore();

  const menuItems = [
    { key: '/', icon: <IconDashboard />, label: '仪表盘' },
    { key: '/tasks', icon: <IconSchedule />, label: '定时任务' },
    { key: '/scripts', icon: <FilePenIcon />, label: '脚本管理' },
    { key: '/env', icon: <KeyIcon />, label: '环境变量' },
    { key: '/dependences', icon: <PackageOpenIcon />, label: '依赖管理' },
    { key: '/subscriptions', icon: <CalendarSyncIcon />, label: '订阅管理' },
    { key: '/logs', icon: <FileClockIcon />, label: '执行日志' },
    { key: '/terminal', icon: <SquareTerminalIcon />, label: '终端' },
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
