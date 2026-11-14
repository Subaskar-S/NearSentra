import { NearProvider } from 'near-connect-hooks';
import { Logo } from '@/components/Logo';
import { SummaryCards } from '@/components/SummaryCards';
import { config } from '@/config';
import { useVault } from '@/hooks/useVault';
import { formatUtc } from '@/lib/format';
import { summarize } from '@/lib/stats';
import { AdminControls } from '@/pages/AdminControls';
import { AgentList } from '@/pages/AgentList';
import { AlertPanel } from '@/pages/AlertPanel';
import { TransactionFeed } from '@/pages/TransactionFeed';

function Dashboard() {
  const { vaultStatus, agents, recentTransfers, loading, error, lastUpdated, refresh } = useVault();
  const summary = summarize(agents, recentTransfers);
  const globallyPaused = vaultStatus?.paused === true;
  return (
    <main className="container py-4">
      <header className="ns-header">
        <div className="ns-brand">
          <Logo />
          <div>
            <h1 className="ns-title">
              Near<span>Sentra</span>
            </h1>
            <div className="ns-tagline">Programmable security for AI agent money on NEAR</div>
          </div>
        </div>
        <div className="ns-meta">
          <span className="ns-pill">
            <span className={`ns-dot ${error || globallyPaused ? 'bad' : ''}`} />
            <strong>{error ? 'chain unreachable' : globallyPaused ? 'vault paused' : 'vault live'}</strong>
          </span>
          <span className="ns-pill">
            vault <strong className="ns-mono">{config.vaultAccountId}</strong>
          </span>
          {vaultStatus && (
            <span className="ns-pill">
              admin <strong className="ns-mono">{vaultStatus.admin_account_id}</strong>
            </span>
          )}
          <span className="ns-pill">{lastUpdated ? `updated ${formatUtc(lastUpdated)}` : 'not loaded yet'}</span>
          <button className="btn btn-sm btn-outline-secondary" onClick={() => void refresh()}>
            Refresh
          </button>
        </div>
      </header>
      {error && <div className="alert alert-danger">Could not refresh from the chain: {error}</div>}
      <SummaryCards summary={summary} />
      <AgentList agents={agents} loading={loading} />
      <TransactionFeed transfers={recentTransfers} />
      <AlertPanel />
      <AdminControls vaultStatus={vaultStatus} agents={agents} onChanged={refresh} />
      <footer className="ns-footer">
        Read straight from the NEAR chain and the Sentinel. This page holds no keys; the vault contract makes every decision.
      </footer>
    </main>
  );
}

export default function App() {
  return (
    <NearProvider config={{ network: config.walletNetwork }}>
      <Dashboard />
    </NearProvider>
  );
}
