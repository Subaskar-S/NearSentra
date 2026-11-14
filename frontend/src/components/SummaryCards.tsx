import { formatNearBig } from '@/lib/format';
import type { Summary } from '@/lib/stats';

function Tile({ label, value, unit, sub, tone }: { label: string; value: string; unit?: string; sub: string; tone?: 'ok' | 'bad' }) {
  return (
    <div className={`ns-tile ${tone ?? ''}`}>
      <div className="ns-tile-label">{label}</div>
      <div className="ns-tile-value">
        {value}
        {unit && <small> {unit}</small>}
      </div>
      <div className="ns-tile-sub">{sub}</div>
    </div>
  );
}

/** Headline numbers. Payment totals cover the events in the transfer feed below. */
export function SummaryCards({ summary }: { summary: Summary }) {
  return (
    <section className="row row-cols-2 row-cols-lg-5 g-3 mb-4" aria-label="Summary">
      <div className="col">
        <Tile
          label="Agents"
          value={String(summary.agents)}
          sub={`${summary.active} active · ${summary.paused} paused · ${summary.revoked} revoked`}
        />
      </div>
      <div className="col">
        <Tile label="Payments approved" value={String(summary.approved)} sub="allowed by the contract" tone="ok" />
      </div>
      <div className="col">
        <Tile label="Payments blocked" value={String(summary.rejected)} sub="refused by the contract" tone="bad" />
      </div>
      <div className="col">
        <Tile label="NEAR paid out" value={formatNearBig(summary.approvedYocto)} unit="NEAR" sub="approved amounts" tone="ok" />
      </div>
      <div className="col">
        <Tile label="NEAR blocked" value={formatNearBig(summary.rejectedYocto)} unit="NEAR" sub="never left the vault" tone="bad" />
      </div>
    </section>
  );
}
