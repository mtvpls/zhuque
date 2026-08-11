import React from 'react';
import './MobileRecordCard.css';

export interface MobileRecordField {
  label: string;
  value: React.ReactNode;
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
        <div className="mobile-record-card__title">{title}</div>
      </div>
      {status && <div className="mobile-record-card__status">{status}</div>}
    </div>
    <div className="mobile-record-card__fields">
      {fields.map((field) => (
        <div
          className={`mobile-record-card__field${field.wide ? ' mobile-record-card__field--wide' : ''}`}
          key={field.label}
        >
          <span className="mobile-record-card__label">{field.label}</span>
          <span className="mobile-record-card__value">{field.value}</span>
        </div>
      ))}
    </div>
    {actions && <div className="mobile-record-card__actions">{actions}</div>}
  </article>
);

export default MobileRecordCard;
