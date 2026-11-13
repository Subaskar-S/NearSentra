# Sentinel event ingestion

NEAR JSON-RPC has no event subscription and no "logs of this contract" query, so the Sentinel reads vault events out
of blocks. Two sources exist (`SENTINEL_EVENT_SOURCE`):

| Source | How it reads | Use for |
|---|---|---|
| `rpc` (default) | `block`, then `chunk`, then `tx` per block. About 2 calls per block on one shard. Only sees transactions addressed to the vault. | Local sandbox, or a node you run |
| `neardata` | One JSON document per block from a FastNEAR neardata server, with every receipt outcome and log on every shard. | Testnet and mainnet |

## What was measured (testnet, November 2026)

- A public testnet RPC returns 10 shards per block: about 11 RPC calls per block, 3.6 calls/s achieved against about
  21 calls/s needed. The `rpc` source cannot keep up there.
- Testnet produces about 105 blocks per minute.
- The free neardata tier is documented at 180 requests/minute but answered 429 after about 30 requests and kept refusing
  for about 40 seconds (it sends `Retry-After: 0`). The sustainable free rate is about 40 blocks per minute, so the
  Sentinel falls behind: lag grew past 250 blocks in a few minutes. **For real testnet or mainnet use set
  `NEARDATA_API_KEY` and raise `NEARDATA_REQUESTS_PER_MINUTE`, or run your own source.** Without that the Sentinel
  reports `/health` 503 and `/status` lag instead of pretending to be current. The API key is sent as a query string and is
  never logged or put in error messages.

## Guarantees

- A block is applied only after it was read completely, so a failed read never delivers half a block and a retry never
  counts an event twice. Each event also carries a stable ID (receipt ID and log index) and repeats are dropped.
- With `CHECKPOINT_PATH` the last processed block and its hash are saved atomically. After a restart the Sentinel resumes
  there and reads the blocks produced meanwhile. A checkpoint whose hash does not match the chain (another network, a
  restarted sandbox), that belongs to another vault, or that is ahead of the chain is ignored. If it is further behind than
  `MAX_CATCHUP_BLOCKS` the Sentinel skips ahead and records an `ingestion_gap` alert.
- Without a checkpoint a restart starts at the head and **does not recover** events from the downtime. A log line says so.
- Events read after an outage or a restart are catch-up: recorded and alerting, but they never trigger a pause, because the
  agent may have stopped long ago.
- Lag is `head - last processed`. `/health` is 503 above `MAX_LAG_BLOCKS` (default 120); `/status` shows the numbers.
- The `rpc` source misses events from receipts sent by other contracts; `neardata` sees all receipts executed on the vault.

## Tests

`services/sentinel/tests/test_ingestion.py` runs the real `poll_once` against a scripted chain: mid-block failure, ordered
retry, duplicate delivery, disconnection and recovery, restart with and without a checkpoint, foreign or stale checkpoints,
lag, the neardata parser, rate limiting and the API key never appearing in errors.
