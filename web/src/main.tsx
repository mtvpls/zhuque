import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { RouterProvider } from 'react-router-dom';
import '@arco-design/web-react/dist/css/arco.css';
import './index.css';
import router from './router';
import { installAuthErrorHandler } from './utils/request';

installAuthErrorHandler();

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <RouterProvider router={router} />
  </StrictMode>
);
