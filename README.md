# Japanese Address Parser API

A high-performance REST API for parsing Japanese addresses, built with Rust and Axum. This service provides structured parsing of Japanese addresses into their constituent parts (prefecture, city, town, and remaining address components).

⚠️ **Note**: This project is a wrapper around the core library [`japanese-address-parser`](https://github.com/YuukiToriyama/japanese-address-parser).

[![Docker Pulls](https://img.shields.io/docker/pulls/framjet/japanese-address-parser-api)](https://hub.docker.com/r/framjet/japanese-address-parser-api)
[![GitHub release](https://img.shields.io/github/release/framjet/rust-japanese-address-parser-api.svg)](https://github.com/framjet/rust-japanese-address-parser-api/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

## Features

- 🚀 **High Performance**: Built with Rust and Axum for maximum throughput
- 🌐 **RESTful API**: Simple HTTP endpoints with JSON responses
- 📊 **Comprehensive Metrics**: Prometheus-compatible metrics endpoint
- 🐳 **Docker Ready**: Multi-architecture Docker images (AMD64, ARM64)
- 🔍 **Health Checks**: Built-in health monitoring
- 📝 **Structured Logging**: JSON-formatted logs with tracing
- 🛡️ **Production Ready**: Graceful shutdown, CORS support, and error handling

## Quick Start

### Using Docker (Recommended)

```bash
# Run the latest version
docker run -p 3000:3000 framjet/japanese-address-parser-api:latest

# Or using docker-compose
curl -O https://raw.githubusercontent.com/framjet/rust-japanese-address-parser-api/main/docker-compose.yml
docker-compose up -d
```

### Building from Source

```bash
# Clone the repository
git clone https://github.com/framjet/rust-japanese-address-parser-api.git
cd rust-japanese-address-parser-api

# Build and run
cargo build --release
cargo run

# Or using cargo-watch for development
cargo install cargo-watch
cargo watch -x run
```

## API Endpoints

### Parse Address

Parse a Japanese address into structured components.

**GET** `/parse?address={address}`

```bash
curl "http://localhost:3000/parse?address=東京都渋谷区神宮前1-1-1"
```

**POST** `/parse`

```bash
curl -X POST http://localhost:3000/parse \
-H "Content-Type: application/json" \
-d '{"address": "東京都渋谷区神宮前1-1-1"}'
```

**Response:**
```json
{
"success": true,
"result": {
"prefecture": "東京都",
"city": "渋谷区",
"town": "神宮前",
"rest": "1-1-1"
},
"processing_time_ms": 15
}
```

### Health Check

Check the service health status.

**GET** `/health`

```bash
curl http://localhost:3000/health
```

**Response:**
```json
{
"status": "healthy",
"service": "japanese-address-parser-api",
"version": "1.0.0",
"timestamp": "2025-01-23T10:30:45Z",
"uptime_seconds": 3600
}
```

### Metrics

Get Prometheus-compatible metrics.

**GET** `/metrics`

```bash
curl http://localhost:3000/metrics
```

## Configuration

The service can be configured using environment variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `HOST` | `0.0.0.0` | Bind address |
| `PORT` | `3000` | Port to listen on |
| `RUST_LOG` | `info` | Log level (error, warn, info, debug, trace) |

### Example with custom configuration

```bash
# Using environment variables
export HOST=127.0.0.1
export PORT=8080
export RUST_LOG=debug
cargo run

# Or with Docker
docker run -p 8080:8080 -e PORT=8080 -e RUST_LOG=debug framjet/japanese-address-parser-api:latest
```

## Docker Deployment

### Docker Compose

```yaml
version: '3.8'
services:
japanese-address-parser:
image: framjet/japanese-address-parser-api:latest
ports:
- "3000:3000"
environment:
- RUST_LOG=info
- HOST=0.0.0.0
- PORT=3000
restart: unless-stopped
healthcheck:
test: ["CMD", "curl", "-f", "http://localhost:3000/health"]
interval: 30s
timeout: 10s
retries: 3
start_period: 40s
```

### Kubernetes

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
name: japanese-address-parser
spec:
replicas: 3
selector:
matchLabels:
app: japanese-address-parser
template:
metadata:
labels:
app: japanese-address-parser
spec:
containers:
- name: api
image: framjet/japanese-address-parser-api:latest
ports:
- containerPort: 3000
env:
- name: RUST_LOG
value: "info"
livenessProbe:
httpGet:
path: /health
port: 3000
initialDelaySeconds: 30
periodSeconds: 10
readinessProbe:
httpGet:
path: /health
port: 3000
initialDelaySeconds: 5
periodSeconds: 5
---
apiVersion: v1
kind: Service
metadata:
name: japanese-address-parser
spec:
selector:
app: japanese-address-parser
ports:
- port: 80
targetPort: 3000
type: LoadBalancer
```

## Monitoring

The service provides comprehensive metrics at `/metrics` endpoint in Prometheus format:

- **Request metrics**: Total requests, success/failure rates, requests by method
- **Performance metrics**: Average, min, max parsing times, response time histograms
- **System metrics**: Service uptime, success rates

### Grafana Dashboard

Import our pre-built Grafana dashboard for monitoring:

```bash
# Download the dashboard JSON
curl -O https://raw.githubusercontent.com/framjet/rust-japanese-address-parser-api/main/grafana-dashboard.json
```

## Development

### Prerequisites

- Rust 1.70+ (2021 edition)
- Docker (optional, for containerized development)

### Local Development

```bash
# Clone and enter the project
git clone https://github.com/framjet/rust-japanese-address-parser-api.git
cd rust-japanese-address-parser-api

# Run in development mode with auto-reload
cargo install cargo-watch
cargo watch -x "run"

# Run tests
cargo test

# Check formatting
cargo fmt --check

# Run clippy for linting
cargo clippy -- -D warnings

# Build for production
cargo build --release
```

### Testing

```bash
# Run all tests
cargo test

# Run tests with output
cargo test -- --nocapture

# Run specific test
cargo test test_parse_post_valid_address

# Run tests with coverage (requires cargo-tarpaulin)
cargo install cargo-tarpaulin
cargo tarpaulin --out Html
```

## Performance

This API is designed for high performance:

- **Async/await**: Full async processing with Tokio runtime
- **Zero-copy parsing**: Efficient string handling where possible
- **Connection pooling**: Reuses connections for better throughput
- **Structured logging**: Minimal performance impact with structured JSON logs
- **Optimized builds**: Release builds use LTO and size optimization

### Benchmarks

On a typical cloud instance (2 vCPU, 4GB RAM):

- **Throughput**: ~10,000 requests/second
- **Latency**: <5ms average response time
- **Memory**: <50MB resident memory usage

## Error Handling

The API provides consistent error responses:

```json
{
"success": false,
"result": null,
"error": "Missing or empty 'address' parameter",
"processing_time_ms": 1
}
```

Common error cases:
- Missing address parameter
- Empty address string
- Invalid JSON payload (POST requests)
- Internal parsing errors

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Contributing

We welcome contributions! Please see our [Contributing Guide](CONTRIBUTING.md) for details.

1. Fork the repository
2. Create your feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add some amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

## Support

- 🐛 **Bug Reports**: [GitHub Issues](https://github.com/framjet/rust-japanese-address-parser-api/issues)
- 💡 **Feature Requests**: [GitHub Discussions](https://github.com/framjet/rust-japanese-address-parser-api/discussions)
- 📖 **Documentation**: [Wiki](https://github.com/framjet/rust-japanese-address-parser-api/wiki)

---

**Made with ❤️ by [FramJet](https://github.com/framjet)**