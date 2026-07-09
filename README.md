# container-stats-exporter

this exporter will polling supported container host, collect stats of its containers.

# host system requirements

MSRV: 1.88
cgroupv2-based container host

# Usage

```
Usage: container-stats-exporter [OPTIONS]

Options:
  -r, --runtime <RUNTIME>
          container runtime
          
          [default: docker]
          [possible values: docker, containerd, podman]

      --runtime_proc <RUNTIME_PROC>
          [default: /tmp/proc]

  -H, --host <HOST>
          default value will connect to OS specific handler
          
          [default: unix:///var/run/docker.sock]

  -b, --bind <BIND>
          HTTP/HTTPS server bind host
          
          [default: 0.0.0.0:12096]

  -s, --secure
          enable HTTPS mode

      --tls_key <TLS_KEY_PATH>
          HTTPS server key path
          
          [default: ./server.key]

      --tls_cert <TLS_CERT_PATH>
          HTTPS server certificate path
          
          [default: ./server.crt]

  -i, --polling_interval <POLLING_MILLIS>
          polling interval in milliseconds
          
          [default: 2000]

  -v, --verbose...
          verbosity output, more v for more verbose

      --namespace <NAMESPACE>
          namespace of the runtime, no effects on docker
          
          [default: default]

  -h, --help
          Print help (see a summary with '-h')
```

# Build requirements

Rust 1.88

# Cross Compile

1. build the builder (do once)
   `docker build -t cts/rust-aarch64-linux-gnu:1.96 -f Dockerfile.toolchain .`
2. build app
   `docker build -f Dockerfile.app --platform linux/arm64 -t cts/container-stats-exporter:latest .`
3. create image backup
   `docker save cts/container-stats-exporter:latest | xz -vvv -T 7 > container-stats-exporter-latest.tar.xz`

# Where is metrics.proto from?

https://github.com/containerd/cgroups/blob/main/cgroup2/stats/metrics.proto

# using docker image backup file

1. `docker load < container-stats-exporter-latest.tar.xz`
2. run container with the following command
  - docker
    ```
    docker run -d \
      --name container-stats-exporter \
      -p 12096:12096 \
      -v /var/run/docker.sock:/var/run/docker.sock \
      --restart unless-stopped \
      --log-driver local \
      cts/container-stats-exporter:latest
    ```
  - containerd, we need set some options to the container
    ```
    nerdctl run -d \
      --name container-stats-exporter \
      -p 12096:12096 \
      -v /var/run/containerd/containerd.sock:/var/run/containerd/containerd.sock \
      -v /proc:/tmp/proc:ro \
      --restart unless-stopped \
      cts/container-stats-exporter:latest \
      --runtime containerd \
      --host unix:///var/run/containerd/containerd.sock
    ```

# Prometheus registry metrics

| Label name | Description |
|------------|-------------|
| id         | Control Group v2 ID that includes container ID, <br />eg. `/system.slice/<runtime>-<namespace>-<very_long_hex_id>.scope` |
| name       | Container name without initial slash |

| Metric Name                         | Type  | Description |
|-------------------------------------|-------|-------------|
| ~~container_cpu_usage_ratios~~      | Gauge | ~~Value of container logical CPU usage, no value when runtime is containerd~~ <br />Deprecated, use `rate(container_cpu_usage_accumulated[1m])` instead |
| container_cpu_usage_accumulated     | Gauge | Value of container CPU usage accumulated since the container is started |
| container_cpu_user_accumulated      | Gauge | Value of container CPU user time accumulated since the container is started |
| container_cpu_system_accumulated    | Gauge | Value of container CPU system time accumulated since the container is started |
| container_cpu_throttled_accumulated | Gauge | Value of container CPU throttled time accumulated since the container is started |
| container_memory_usage_bytes        | Gauge | Value of container memory usage in bytes |
| container_memory_limit_bytes        | Gauge | Value of container memory limitation in bytes |
| container_memory_swap_usage_bytes   | Gauge | Value of container swap memory usage in bytes |
| container_memory_swap_limit_bytes   | Gauge | Value of container swap memory limitation in bytes |
| container_memory_pgfault            | Gauge | Value of container memory page faults since the container is started |
| container_memory_pgmajfault         | Gauge | Value of container memory major page faults since the container is started |
| container_network_receive_bytes     | Gauge | Value of container received data from network data in bytes |
| container_network_transmit_bytes    | Gauge | Value of container sent data from network in bytes |
| container_network_receive_packets   | Gauge | Value of container received network packets since the container is started |
| container_network_transmit_packets  | Gauge | Value of container sent network packets since the container is started |
| container_network_receive_bps       | Gauge | Value of container network receive throughput in bps |
| container_network_transmit_bps      | Gauge | Value of container network sent throughput in bps |
| container_blkio_receive_bytes       | Gauge | Value of container read data from blkio in bytes |
| container_blkio_transmit_bytes      | Gauge | Value of container write data to blkio in bytes |
| container_blkio_receive_ios         | Gauge | Value of container read data from blkio in I/O operations |
| container_blkio_transmit_ios        | Gauge | Value of container write data to blkio in I/O operations |
| container_blkio_receive_byteps      | Gauge | Value of container blkio receive throughput in byte per second |
| container_blkio_transmit_byteps     | Gauge | Value of container blkio sent throughput in byte per second |
| container_pids                      | Gauge | Value of container pids |

# Note

Since internal polling period will never matches prometheus polling period, period-type data such as throughputs and CPU usage are just for reference only.

# todo

- ability of push metrics to Push Gateway
- podman container runtime support
