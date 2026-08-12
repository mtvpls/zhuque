import React, { useLayoutEffect, useRef, useState } from 'react';
import { Tooltip } from '@arco-design/web-react';
import './MobileRecordCard.css';

export interface MobileRecordField {
  label: string;
  value: React.ReactNode;
  tooltip?: React.ReactNode;
  wide?: boolean;
}

interface MobileRecordCardProps {
  eyebrow?: React.ReactNode;
  title: React.ReactNode;
  status?: React.ReactNode;
  fields: MobileRecordField[];
  actions?: React.ReactNode;
  className?: string;
}

interface OverflowTooltipProps {
  children: React.ReactNode;
  className: string;
  content?: React.ReactNode;
}

const isOverflowing = (element: HTMLElement) => {
  const elements = [element, ...Array.from(element.querySelectorAll<HTMLElement>('*'))];
  return elements.some((item) => item.scrollWidth > item.clientWidth + 1);
};

const OverflowTooltip: React.FC<OverflowTooltipProps> = ({ children, className, content }) => {
  const targetRef = useRef<HTMLSpanElement>(null);
  const [truncated, setTruncated] = useState(false);
  const [fullText, setFullText] = useState<React.ReactNode>(content || '');

  useLayoutEffect(() => {
    const checkOverflow = () => {
      const target = targetRef.current;
      if (!target) return;
      setTruncated(isOverflowing(target));
      if (content === undefined) {
        setFullText(target.innerText || target.textContent || '');
      }
    };

    const frame = window.requestAnimationFrame(checkOverflow);
    const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(checkOverflow);
    if (observer && targetRef.current) observer.observe(targetRef.current);

    return () => {
      window.cancelAnimationFrame(frame);
      observer?.disconnect();
    };
  }, [children, content]);

  const trigger = (
    <span ref={targetRef} className={className}>
      {children}
    </span>
  );

  return truncated ? (
    <Tooltip content={content ?? fullText} position="top">
      {trigger}
    </Tooltip>
  ) : trigger;
};

const MobileRecordField: React.FC<{ field: MobileRecordField }> = ({ field }) => (
  <div className={`mobile-record-card__field${field.wide ? ' mobile-record-card__field--wide' : ''}`}>
    <span className="mobile-record-card__label">{field.label}</span>
    <span className="mobile-record-card__value">
      <OverflowTooltip className="mobile-record-card__tooltip-trigger" content={field.tooltip}>
        {field.value}
      </OverflowTooltip>
    </span>
  </div>
);

const MobileRecordCard: React.FC<MobileRecordCardProps> = ({
  eyebrow,
  title,
  status,
  fields,
  actions,
  className = '',
}) => (
  <article className={`mobile-record-card ${className}`}>
    <div className="mobile-record-card__header">
      <div className="mobile-record-card__heading">
        {eyebrow && <div className="mobile-record-card__eyebrow">{eyebrow}</div>}
        <OverflowTooltip className="mobile-record-card__title">{title}</OverflowTooltip>
      </div>
      {status && <div className="mobile-record-card__status">{status}</div>}
    </div>
    <div className="mobile-record-card__fields">
      {fields.map((field) => <MobileRecordField key={field.label} field={field} />)}
    </div>
    {actions && <div className="mobile-record-card__actions">{actions}</div>}
  </article>
);

export default MobileRecordCard;
