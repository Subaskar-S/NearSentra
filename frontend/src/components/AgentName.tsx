import { labelFor } from '@/lib/labels';

/** An account shown as its friendly name (when configured) over the real account ID. */
export function AgentName({ id }: { id: string }) {
  const label = labelFor(id);
  if (!label) return <span className="ns-mono">{id}</span>;
  return (
    <span className="d-inline-flex flex-column lh-sm">
      <span className="fw-semibold">{label}</span>
      <span className="ns-mono">{id}</span>
    </span>
  );
}
