# Build requirements

Rust 1.87

# Usage

```
Usage: docker-stat-prom [OPTIONS]

Options:
  -H, --host <HOST>                        docker host [default: unix:///var/run/docker.sock]
  -b, --bind <BIND>                        HTTP/HTTPS server bind host [default: 0.0.0.0:12096]
  -s, --secure                             enable HTTPS mode
      --tls_key <TLS_KEY_PATH>             HTTPS server key path [default: ./server.key]
      --tls_cert <TLS_CERT_PATH>           HTTPS server certificate path [default: ./server.crt]
  -i, --polling_interval <POLLING_MILLIS>  polling interval in milliseconds [default: 2000]
  -h, --help                               Print help (see more with '--help')
```

# host system requirements

cgroup v2

# Cross Compile

1. build the builder (do once)
   `docker build -t cts/rust-aarch64-linux-gnu:1.87 -f Dockerfile.toolchain .`
2. build app
   `docker build --platform linux/arm64 -t cts/docker-stat-prom:latest .`
3. create image backup
   `docker save cts/docker-stat-prom:latest | xz -vvv -T 7 > docker-stat-prom-latest.tar.xz`

# using docker image backup file

1. `sudo docker load < docker-stat-prom-latest.tar.xz`
2. `sudo docker run -d --name docker-stat-prom -p 12096:12096 -v /var/run/docker.sock:/var/run/docker.sock --restart unless-stopped --log-driver local cts/docker-stat-prom:latest`

# Prometheus registry metrics

| Label name | Description |
|------------|-------------|
| id         | Control Group v2 ID that includes container ID, <br />eg. `/system.slice/docker-<very_long_hex_id>.scope` |
| name       | Container name without initial slash |

| Metric Name                      | Type  | Description |
|----------------------------------|-------|-------------|
| container_cpu_usage_ratios       | Gauge | Value of container logical CPU usage |
| container_memory_usage_bytes     | Gauge | Value of container memory usage in bytes |
| container_memory_limit_bytes     | Gauge | Value of container memory limitation in bytes |
| container_network_receive_bytes  | Gauge | Value of container received data from network data in bytes |
| container_network_transmit_bytes | Gauge | Value of container sent data from network in bytes |
| container_blkio_receive_bytes    | Gauge | Value of container read data from blkio in bytes |
| container_blkio_transmit_bytes   | Gauge | Value of container write data to blkio in bytes |
| container_network_receive_bps    | Gauge | Value of container network receive throughput in bps |
| container_network_transmit_bps   | Gauge | Value of container network sent throughput in bps |
| container_blkio_receive_byteps   | Gauge | Value of container blkio receive throughput in byte per second |
| container_blkio_transmit_byteps  | Gauge | Value of container blkio sent throughput in byte per second |

# Note

Since internal polling period will never matches prometheus polling period, period-type data such as throughputs and CPU usage are just for reference only.

# todo

- push metrics
- read cgroup v2 directly, to support such as swap usage