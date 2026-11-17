import { useState, type ReactNode } from 'react';

type Props = {
  title: string;
  busy?: boolean;
  children: ReactNode;
  apply: () => void;
  cancel: () => void;
};

export default function StepEditDialog(p: Props) {
  const [error, setError] = useState('');
  return <form className="step-edit" aria-label={p.title} onSubmit={event => {
    event.preventDefault();
    try { p.apply(); setError(''); } catch (error) { setError(String(error)); }
  }}>
    <strong>{p.title}</strong>
    {p.children}
    {error && <p className="inline-error" role="alert">{error}</p>}
    <div className="input-action">
      <button className="primary" disabled={p.busy} type="submit">应用规则和输入</button>
      <button type="button" disabled={p.busy} onClick={p.cancel}>取消草稿</button>
    </div>
  </form>;
}
