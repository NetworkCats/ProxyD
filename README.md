# ProxyD

[![CI](https://github.com/NetworkCats/ProxyD/actions/workflows/ci.yml/badge.svg)](https://github.com/NetworkCats/ProxyD/actions/workflows/ci.yml)
[![CodeQL](https://github.com/NetworkCats/ProxyD/actions/workflows/codeql.yml/badge.svg)](https://github.com/NetworkCats/ProxyD/actions/workflows/codeql.yml)
[![codecov](https://codecov.io/gh/NetworkCats/ProxyD/branch/main/graph/badge.svg)](https://codecov.io/gh/NetworkCats/ProxyD)

IP reputation API service with gRPC and REST interfaces.

## Features

- Query IP reputation via REST or gRPC
- Supports IPv4 and IPv6, including CIDR ranges
- Automatic daily sync from OpenProxyDB (02:00 UTC)
- LMDB storage for optimal performance
- Prometheus metrics endpoint

## Quick Start

```bash
docker run -d \
  --name proxyd \
  -p 7891:7891 \
  -p 7892:7892 \
  -v proxyd-data:/data \
  networkcat/proxyd
```

## API

### REST (port 7891)

```bash
# Query single IP
curl http://localhost:7891/v1/ip/1.0.0.13

# Query CIDR range
curl "http://localhost:7891/v1/range?cidr=1.0.0.0/24"

# Batch IP lookup
curl -X POST -H "Content-Type: application/json" \
  -d '{"ips": ["8.8.8.8", "1.1.1.1"]}' \
  http://localhost:7891/v1/ip/batch

# Batch range lookup
curl -X POST -H "Content-Type: application/json" \
  -d '{"cidrs": ["8.8.8.0/24", "1.1.1.0/24"]}' \
  http://localhost:7891/v1/range/batch

# Health check
curl http://localhost:7891/health

# Metrics
curl http://localhost:7891/metrics
```

### gRPC (port 7892)

```protobuf
service ProxyD {
  rpc LookupIP(IPRequest) returns (ReputationResponse);
  rpc LookupRange(RangeRequest) returns (ReputationResponse);
  rpc BatchLookupIP(BatchIPRequest) returns (BatchReputationResponse);
  rpc BatchLookupRange(BatchRangeRequest) returns (BatchReputationResponse);
}
```

## Configuration

| Environment Variable | Default | Description |
|---------------------|---------|-------------|
| `PROXYD_DATA_DIR` | `/data` | Data directory path |
| `PROXYD_REST_PORT` | `7891` | REST API port |
| `PROXYD_GRPC_PORT` | `7892` | gRPC API port |
| `PROXYD_SYNC_HOUR_UTC` | `2` | Daily sync hour (UTC) |
| `PROXYD_CSV_URL` | OpenProxyDB URL | CSV source URL |

## Build

```bash
cargo build --release
```

## License

MIT
