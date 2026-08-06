import React, { useEffect, useRef, useState } from 'react';
import { Terminal as XTerm } from 'xterm';
import { FitAddon } from 'xterm-addon-fit';
import { WebLinksAddon } from 'xterm-addon-web-links';
import { Button, Space, Message } from '@arco-design/web-react';
import {
  IconArrowDown,
  IconArrowLeft,
  IconArrowRight,
  IconArrowUp,
  IconFullscreen,
  IconFullscreenExit,
  IconHome,
  IconRefresh,
  IconDelete,
  IconClose,
} from '@arco-design/web-react/icon';
import 'xterm/css/xterm.css';
import './Terminal.less';

const Terminal: React.FC = () => {
  const userAgent = navigator.userAgent;
  const browserClass = /Via/i.test(userAgent)
    ? 'browser-via'
    : /Firefox/i.test(userAgent)
      ? 'browser-firefox'
      : /Chrome/i.test(userAgent)
        ? 'browser-chrome'
        : 'browser-other';

  const terminalRef = useRef<HTMLDivElement>(null);
  const xtermRef = useRef<XTerm | null>(null);
  const wsRef = useRef<WebSocket | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const terminalContainerRef = useRef<HTMLDivElement>(null);
  const modifiersRef = useRef({ ctrl: false, alt: false });
  const directionHoldRef = useRef<ReturnType<typeof window.setInterval> | null>(null);
  const directionHoldTimeoutRef = useRef<ReturnType<typeof window.setTimeout> | null>(null);
  const baseViewportHeightRef = useRef(0);
  const fullscreenIntentRef = useRef(false);
  const [connected, setConnected] = useState(false);
  const [isFullscreen, setIsFullscreen] = useState(false);
  const [, setIsMobile] = useState(false);
  const [, setModifierVersion] = useState(0);

  const sendInput = (data: string) => {
    if (wsRef.current?.readyState === WebSocket.OPEN) {
      wsRef.current.send(JSON.stringify({ type: 'input', data }));
    }
  };

  const applyModifiers = (data: string) => {
    if (data.length === 1) {
      const code = data.toLowerCase().charCodeAt(0);
      if (modifiersRef.current.ctrl && code >= 97 && code <= 122) {
        return String.fromCharCode(code - 96);
      }
      if (modifiersRef.current.alt && code >= 32 && code <= 126) {
        return '\x1b' + data;
      }
    }
    return data;
  };

  const connectTerminal = () => {
    if (!terminalRef.current) return;

    // 检测是否为移动设备
    // 初始化 xterm.js
    const term = new XTerm({
      cursorBlink: true,
      fontSize: window.innerWidth <= 768 ? 12 : 14,
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
      setConnected(false);
    };

    ws.onclose = () => {
      setConnected(false);
    };

    // 监听用户输入
    term.onData((data) => {
      sendInput(applyModifiers(data));

      if (data.length > 0 && (modifiersRef.current.ctrl || modifiersRef.current.alt)) {
        modifiersRef.current.ctrl = false;
        modifiersRef.current.alt = false;
        setModifierVersion((value) => value + 1);
      }
    });

    // 窗口调整，保留 xterm 当前是否位于输出底部的状态。
    const fitTerminal = () => {
      const activeBuffer = term.buffer.active;
      const wasAtBottom = activeBuffer.viewportY >= activeBuffer.baseY;
      fitAddon.fit();
      if (wasAtBottom) {
        term.scrollToBottom();
      }
    };

    const handleResize = () => {
      setTimeout(() => {
        fitTerminal();
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
    return () => {
      if (directionHoldRef.current !== null) {
        window.clearInterval(directionHoldRef.current);
        directionHoldRef.current = null;
      }
      if (directionHoldTimeoutRef.current !== null) {
        window.clearTimeout(directionHoldTimeoutRef.current);
        directionHoldTimeoutRef.current = null;
      }
    };
  }, []);

  useEffect(() => {
    const html = document.documentElement;
    const body = document.body;
    if (!isFullscreen) return;

    const previous = {
      htmlOverflow: html.style.overflow,
      htmlOverscrollBehavior: html.style.overscrollBehavior,
      bodyOverflow: body.style.overflow,
      bodyOverscrollBehavior: body.style.overscrollBehavior,
    };

    html.style.overflow = 'hidden';
    html.style.overscrollBehavior = 'none';
    body.style.overflow = 'hidden';
    body.style.overscrollBehavior = 'none';

    return () => {
      html.style.overflow = previous.htmlOverflow;
      html.style.overscrollBehavior = previous.htmlOverscrollBehavior;
      body.style.overflow = previous.bodyOverflow;
      body.style.overscrollBehavior = previous.bodyOverscrollBehavior;
    };
  }, [isFullscreen]);

  useEffect(() => {
    const updateViewport = () => setIsMobile(window.innerWidth <= 768 || navigator.maxTouchPoints > 0 || 'ontouchstart' in window);
    const updateFullscreen = () => {
      requestAnimationFrame(updateKeyboardInset);
    };
    const updateKeyboardInset = () => {
      const viewport = window.visualViewport;
      const container = terminalContainerRef.current;
      if (!container) return;

      const viewportHeight = viewport?.height ?? window.innerHeight;
      if (
        baseViewportHeightRef.current === 0 ||
        viewportHeight >= baseViewportHeightRef.current - 24
      ) {
        baseViewportHeightRef.current = viewportHeight;
      }
      const viewportDelta = baseViewportHeightRef.current - viewportHeight;
      const inset = viewportDelta > 120 ? viewportDelta : 0;

      if (fullscreenIntentRef.current) {
        container.style.setProperty('--terminal-viewport-height', String(viewportHeight) + 'px');
      } else {
        container.style.removeProperty('--terminal-viewport-height');
        baseViewportHeightRef.current = viewportHeight;
      }

      container.style.setProperty('--terminal-keyboard-inset', String(inset) + 'px');
    };
    updateViewport();
    updateKeyboardInset();
    document.addEventListener('fullscreenchange', updateFullscreen);
    window.addEventListener('resize', updateViewport);
    window.addEventListener('resize', updateKeyboardInset);
    window.visualViewport?.addEventListener('resize', updateKeyboardInset);
    window.visualViewport?.addEventListener('scroll', updateKeyboardInset);

    const cleanup = connectTerminal();

    // 添加额外的 resize 监听，确保在布局变化时调整终端大小
    const resizeObserver = new ResizeObserver(() => {
      if (fitAddonRef.current && xtermRef.current) {
        setTimeout(() => {
          const activeBuffer = xtermRef.current?.buffer.active;
          const wasAtBottom = activeBuffer
            ? activeBuffer.viewportY >= activeBuffer.baseY
            : false;
          fitAddonRef.current?.fit();
          if (wasAtBottom) {
            xtermRef.current?.scrollToBottom();
          }
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
      document.removeEventListener('fullscreenchange', updateFullscreen);
      window.removeEventListener('resize', updateViewport);
      window.removeEventListener('resize', updateKeyboardInset);
      window.visualViewport?.removeEventListener('resize', updateKeyboardInset);
      window.visualViewport?.removeEventListener('scroll', updateKeyboardInset);
      cleanup?.();
    };
  }, []);

  const toggleFullscreen = async () => {
    try {
      if (isFullscreen) {
        if (document.fullscreenElement) {
          await document.exitFullscreen();
        }
        fullscreenIntentRef.current = false;
        terminalContainerRef.current?.style.removeProperty('--terminal-viewport-height');
        setIsFullscreen(false);
        screen.orientation.unlock();
      } else {
        fullscreenIntentRef.current = true;
        const fullscreenHeight = window.visualViewport?.height ?? window.innerHeight;
        baseViewportHeightRef.current = fullscreenHeight;
        terminalContainerRef.current?.style.setProperty(
          '--terminal-viewport-height',
          `${fullscreenHeight}px`
        );
        setIsFullscreen(true);
      }
    } catch {
      Message.error('当前浏览器不支持全屏');
    }
  };

  const refocusTerminal = () => {
    const input = terminalRef.current?.querySelector('textarea');
    input?.focus({ preventScroll: true });
    xtermRef.current?.focus();
    requestAnimationFrame(() => {
      terminalRef.current?.querySelector('textarea')?.focus({ preventScroll: true });
      xtermRef.current?.focus();
    });
  };

  const handleShortcut = (data: string) => {
    sendInput(data);
    refocusTerminal();
  };

  const stopDirectionHold = () => {
    if (directionHoldRef.current !== null) {
      window.clearInterval(directionHoldRef.current);
      directionHoldRef.current = null;
    }
    if (directionHoldTimeoutRef.current !== null) {
      window.clearTimeout(directionHoldTimeoutRef.current);
      directionHoldTimeoutRef.current = null;
    }
  };

  const startDirectionHold = (data: string) => {
    stopDirectionHold();
    handleShortcut(data);
    directionHoldTimeoutRef.current = window.setTimeout(() => {
      directionHoldTimeoutRef.current = null;
      directionHoldRef.current = window.setInterval(() => sendInput(data), 140);
    }, 300);
  };

  const toggleModifier = (modifier: 'ctrl' | 'alt') => {
    modifiersRef.current[modifier] = !modifiersRef.current[modifier];
    setModifierVersion((value) => value + 1);
    refocusTerminal();
  };

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
    <div
      ref={terminalContainerRef}
      className={'terminal-container ' + browserClass + (isFullscreen ? ' is-fullscreen' : '')}
    >
      <div className="terminal-toolbar">
        <Space size="small" wrap className="terminal-toolbar-actions">
          <Button
            type="primary"
            size="small"
            icon={<IconRefresh />}
            onClick={handleReconnect}
            disabled={connected}
            title="重连"
            aria-label="重连"
            className="terminal-action-button"
          >
            <span className="terminal-button-label">重连</span>
          </Button>
          <Button
            size="small"
            icon={<IconDelete />}
            onClick={handleClear}
            title="清屏"
            aria-label="清屏"
            className="terminal-action-button"
          >
            <span className="terminal-button-label">清屏</span>
          </Button>
          <Button
            size="small"
            icon={isFullscreen ? <IconFullscreenExit /> : <IconFullscreen />}
            onClick={toggleFullscreen}
            title={isFullscreen ? '退出全屏' : '全屏'}
            aria-label={isFullscreen ? '退出全屏' : '全屏'}
            className="terminal-action-button"
          >
            <span className="terminal-button-label">{isFullscreen ? '退出全屏' : '全屏'}</span>
          </Button>
          <Button
            size="small"
            status="danger"
            icon={<IconClose />}
            onClick={handleDisconnect}
            disabled={!connected}
            title="断开"
            aria-label="断开"
            className="terminal-action-button"
          >
            <span className="terminal-button-label">断开</span>
          </Button>
        </Space>
        <div className="terminal-status">
          <span className={`status-indicator ${connected ? 'connected' : 'disconnected'}`} />
          <span>{connected ? '已连接' : '未连接'}</span>
        </div>
      </div>
      <div ref={terminalRef} className="terminal-content" />
      {isFullscreen && (
        <div
          className="terminal-shortcuts"
          role="toolbar"
          aria-label="终端快捷键"
          onPointerDown={(event) => event.preventDefault()}
        >
          <button type="button" onClick={() => handleShortcut('\x1b')} title="Escape">Esc</button>
          <button type="button" onClick={() => handleShortcut('/')} title="斜杠">/</button>
          <button type="button" onClick={() => handleShortcut('|')} title="竖线">|</button>
          <button type="button" onClick={() => handleShortcut('\x1b[H')} title="Home"><IconHome /></button>
          <button type="button" onClick={() => handleShortcut('\x1b[6~')} title="Page Down">PgDn</button>
          <button
            type="button"
            onPointerDown={(event) => {
              event.preventDefault();
              startDirectionHold('\x1b[A');
            }}
            onPointerUp={stopDirectionHold}
            onPointerCancel={stopDirectionHold}
            onPointerLeave={stopDirectionHold}
            title="上方向（长按连发）"
          ><IconArrowUp /></button>
          <button type="button" onClick={() => handleShortcut('\x1b[F')} title="End">End</button>
          <button type="button" className={modifiersRef.current.ctrl ? 'is-active' : ''} onClick={() => toggleModifier('ctrl')} title="切换 Ctrl 状态">Ctrl</button>
          <button type="button" className={modifiersRef.current.alt ? 'is-active' : ''} onClick={() => toggleModifier('alt')} title="切换 Alt 状态">Alt</button>
          <button type="button" onClick={() => handleShortcut('\t')} title="Tab">Tab</button>
          <button type="button" onClick={() => handleShortcut('\x1b[5~')} title="Page Up">PgUp</button>
          <button
            type="button"
            onPointerDown={(event) => {
              event.preventDefault();
              startDirectionHold('\x1b[D');
            }}
            onPointerUp={stopDirectionHold}
            onPointerCancel={stopDirectionHold}
            onPointerLeave={stopDirectionHold}
            title="左方向（长按连发）"
          ><IconArrowLeft /></button>
          <button
            type="button"
            onPointerDown={(event) => {
              event.preventDefault();
              startDirectionHold('\x1b[B');
            }}
            onPointerUp={stopDirectionHold}
            onPointerCancel={stopDirectionHold}
            onPointerLeave={stopDirectionHold}
            title="下方向（长按连发）"
          ><IconArrowDown /></button>
          <button
            type="button"
            onPointerDown={(event) => {
              event.preventDefault();
              startDirectionHold('\x1b[C');
            }}
            onPointerUp={stopDirectionHold}
            onPointerCancel={stopDirectionHold}
            onPointerLeave={stopDirectionHold}
            title="右方向（长按连发）"
          ><IconArrowRight /></button>
        </div>
      )}
    </div>
  );
};

export default Terminal;
