import { createBrowserRouter, Navigate } from 'react-router-dom';
import BasicLayout from '@/layouts/BasicLayout';
import Login from '@/pages/Login';
import InitialSetup from '@/pages/InitialSetup';
import Dashboard from '@/pages/Dashboard';
import Tasks from '@/pages/Tasks';
import Scripts from '@/pages/Scripts';
import Env from '@/pages/Env';
import Dependences from '@/pages/Dependences';
import Subscriptions from '@/pages/Subscriptions';
import Logs from '@/pages/Logs';
import Config from '@/pages/Config';
import Terminal from '@/pages/Terminal';

const router = createBrowserRouter([
  {
    path: '/setup',
    element: <InitialSetup />,
  },
  {
    path: '/login',
    element: <Login />,
  },
  {
    path: '/',
    element: <BasicLayout />,
    children: [
      {
        index: true,
        element: <Dashboard />,
      },
      {
        path: 'tasks',
        element: <Tasks />,
      },
      {
        path: 'scripts',
        element: <Scripts />,
      },
      {
        path: 'env',
        element: <Env />,
      },
      {
        path: 'dependences',
        element: <Dependences />,
      },
      {
        path: 'subscriptions',
        element: <Subscriptions />,
      },
      {
        path: 'logs',
        element: <Logs />,
      },
      {
        path: 'config',
        element: <Config />,
      },
      {
        path: 'terminal',
        element: <Terminal />,
      },
    ],
  },
  {
    path: '*',
    element: <Navigate to="/" replace />,
  },
]);

export default router;
