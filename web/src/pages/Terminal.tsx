import React, { useEffect, useRef, useState } from 'react';
import { Terminal as XTerm } from 'xterm';
import { FitAddon } from 'xterm-addon-fit';
import { WebLinksAddon } from 'xterm-addon-web-links';
import { Button, Space, Message } from '@arco-design/web-react';
import { IconRefresh, IconDelete, IconClose } from '@arco-design/web-react/icon';
import 'xterm/css/xterm.css';
import './Terminal.less';

const Terminal: React.FC = () => {
  const terminalRef = useRef<HTMLDivElement>(null);
  const xtermRef = useRef<XTerm | null>(null);
  const wsRef = useRef<WebSocket | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const [connected, setConnected] = useState(false);

  const connectTerminal = () => {
    if (!terminalRef.current) return;

    // 检测是否为移动设备
    const isMobile = window.innerWidth <= 768;

    // 初始化 xterm.js
    const term = new XTerm({
      cursorBlink: true,
      fontSize: isMobile ? 12 : 14,
      fontFamily: 'Menlo, Monaco, "Courier New", monospace',
      theme: {
        background: '#1e1e1e',
        foreground: '#d4d4d4',
        cursor: '#d4d4d4',
        black: '#000000',
        red: '#cd3131',
        green: '#0dbc79',
        yellow: '#e5e510',
        blue: '#2472c8',
        magenta: '#bc3fbc',
        cyan: '#11a8cd',
        white: '#e5e5e5',
        brightBlack: '#666666',
        brightRed: '#f14c4c',
        brightGreen: '#23d18b',
        brightYellow: '#f5f543',
        brightBlue: '#3b8eea',
        brightMagenta: '#d670d6',
        brightCyan: '#29b8db',
        brightWhite: '#e5e5e5',
      },
      scrollback: 1000,
      convertEol: true,
    });

    const fitAddon = new FitAddon();
    const webLinksAddon = new WebLinksAddon();

    term.loadAddon(fitAddon);
    term.loadAddon(webLinksAddon);
    term.open(terminalRef.current);

    // 延迟调用 fit 确保容器已完全渲染
    setTimeout(() => {
      fitAddon.fit();
    }, 100);

    xtermRef.current = term;
    fitAddonRef.current = fitAddon;

    // 连接 WebSocket
    const token = localStorage.getItem('token');
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const wsUrl = `${protocol}//${window.location.host}/api/terminal/connect?token=${token}`;
    const ws = new WebSocket(wsUrl);

    ws.onopen = () => {
      setConnected(true);
      Message.success('终端已连接');

      // 连接成功后再次调整大小并发送到服务器
      setTimeout(() => {
        fitAddon.fit();
        if (ws.readyState === WebSocket.OPEN) {
          ws.send(
            JSON.stringify({
              type: 'resize',
              rows: term.rows,
              cols: term.cols,
            })
          );
        }
      }, 100);
    };

    ws.onmessage = async (event) => {
      // 处理文本消息（可能是 session 初始化消息）
      if (typeof event.data === 'string') {
        try {
          const data = JSON.parse(event.data);
          if (data.type === 'session') {
            // 忽略 session 消息，不显示在终端
            console.log('Terminal session ID:', data.id);
            return;
          }
        } catch (e) {
          // 不是 JSON 格式，作为普通文本输出
          term.write(event.data);
          return;
        }
      }

      // 处理二进制消息（PTY 输出）
      if (event.data instanceof Blob) {
        const text = await event.data.text();
        term.write(text);
      } else if (event.data instanceof ArrayBuffer) {
        const text = new TextDecoder().decode(event.data);
        term.write(text);
      } else {
        // 其他类型直接写入
        term.write(event.data);
      }
    };

    ws.onerror = (error) => {
      console.error('WebSocket error:', error);
      Message.error('终端连接错误');
      setConnected(false);
    };

    ws.onclose = () => {
      setConnected(false);
      Message.info('终端连接已断开');
    };

    // 监听用户输入
    term.onData((data) => {
      if (ws.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify({ type: 'input', data }));
      }
    });

    // 窗口调整
    const handleResize = () => {
      setTimeout(() => {
        fitAddon.fit();
        if (ws.readyState === WebSocket.OPEN) {
          ws.send(
            JSON.stringify({
              type: 'resize',
              rows: term.rows,
              cols: term.cols,
            })
          );
        }
      }, 100);
    };

    window.addEventListener('resize', handleResize);

    // 监听屏幕方向变化（移动端）
    window.addEventListener('orientationchange', handleResize);

    wsRef.current = ws;

    return () => {
      window.removeEventListener('resize', handleResize);
      window.removeEventListener('orientationchange', handleResize);
      ws.close();
      term.dispose();
    };
  };

  useEffect(() => {
    const cleanup = connectTerminal();

    // 添加额外的 resize 监听，确保在布局变化时调整终端大小
    const resizeObserver = new ResizeObserver(() => {
      if (fitAddonRef.current && xtermRef.current) {
        setTimeout(() => {
          fitAddonRef.current?.fit();
          if (wsRef.current?.readyState === WebSocket.OPEN) {
            wsRef.current.send(
              JSON.stringify({
                type: 'resize',
                rows: xtermRef.current?.rows,
                cols: xtermRef.current?.cols,
              })
            );
          }
        }, 100);
      }
    });

    if (terminalRef.current) {
      resizeObserver.observe(terminalRef.current);
    }

    return () => {
      resizeObserver.disconnect();
      cleanup?.();
    };
  }, []);

  const handleReconnect = () => {
    if (wsRef.current) {
      wsRef.current.close();
    }
    if (xtermRef.current) {
      xtermRef.current.dispose();
    }
    // 延迟重连，确保清理完成
    setTimeout(() => {
      connectTerminal();
    }, 200);
  };

  const handleClear = () => {
    if (xtermRef.current) {
      xtermRef.current.clear();
    }
  };

  const handleDisconnect = () => {
    if (wsRef.current) {
      wsRef.current.close();
    }
  };

  return (
    <div className="terminal-container">
      <div className="terminal-toolbar">
        <Space size="small" wrap>
          <Button
            type="primary"
            size="small"
            icon={<IconRefresh />}
            onClick={handleReconnect}
            disabled={connected}
          >
            重连
          </Button>
          <Button size="small" icon={<IconDelete />} onClick={handleClear}>
            清屏
          </Button>
          <Button
            size="small"
            status="danger"
            icon={<IconClose />}
            onClick={handleDisconnect}
            disabled={!connected}
          >
            断开
          </Button>
        </Space>
        <div className="terminal-status">
          <span className={`status-indicator ${connected ? 'connected' : 'disconnected'}`} />
          <span>{connected ? '已连接' : '未连接'}</span>
        </div>
      </div>
      <div ref={terminalRef} className="terminal-content" />
    </div>
  );
};

export default Terminal;
