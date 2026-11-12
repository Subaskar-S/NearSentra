import { NearProvider } from 'near-connect-hooks';
import { config } from '@/config';
import { useVault } from '@/hooks/useVault';
import { formatUtc } from '@/lib/format';
import { AdminControls } from '@/pages/AdminControls';
import { AgentList } from '@/pages/AgentList';
import { AlertPanel } from '@/pages/AlertPanel';
import { TransactionFeed } from '@/pages/TransactionFeed';

function Dashboard() {
  const { vaultStatus, agents, recentTransfers, loading, error, lastUpdated, refresh } = useVault();
  return (
    <main className="container py-4">
      <header className="d-flex justify-content-between align-items-end mb-4">
        <div>
          <h1 className="h3 mb-0">NearSentra</h1>
          <div className="text-muted small">
            Vault <span className="font-monospace">{config.vaultAccountId}</span>
            {vaultStatus && (
              <>
                {' '}
                · admin <span className="font-monospace">{vaultStatus.admin_account_id}</span>
                {' · '}
                {vaultStatus.paused ? <strong className="text-danger">GLOBALLY PAUSED</strong> : 'accepting requests'}
              </>
            )}
          </div>
        </div>
        <div className="text-muted small text-end">
          {lastUpdated ? `Updated ${formatUtc(lastUpdated)}` : 'Not loaded yet'}
          <button className="btn btn-sm btn-outline-secondary ms-2" onClick={() => void refresh()}>
            Refresh
          </button>
        </div>
      </header>
      {error && <div className="alert alert-danger">Could not refresh from the chain: {error}</div>}
      <AgentList agents={agents} loading={loading} />
      <TransactionFeed transfers={recentTransfers} />
      <AlertPanel />
      <AdminControls vaultStatus={vaultStatus} agents={agents} onChanged={refresh} />
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
