use std::{
    collections::HashMap,
    io,
    sync::{Arc, OnceLock},
    time::{Duration, SystemTime},
};

use actix_web::http::Uri;
use bollard::{
    API_DEFAULT_VERSION, Docker,
    plugin::{
        ContainerBlkioStats, ContainerCpuStats, ContainerNetworkStats, ContainerStatsResponse,
    },
    query_parameters::{ListContainersOptionsBuilder, StatsOptionsBuilder},
};
use containerd_client::{
    services::v1::{ListContainersRequest, ListTasksRequest, MetricsRequest},
    tonic::transport::Channel as ContainerdChannel,
};
use containerd_client::{tonic::Request, with_namespace};
use futures_util::TryStreamExt;
use prometheus_client::registry::Registry;
use prost::Message;
use serde::Serialize;
use tokio::sync::Mutex;
use tracing::*;

use crate::{
    docker_stat_metrics::DockerStatContainerMetrics,
    usecases::cgroups_v2::{IoEntry, NetworkStat},
};

pub mod cgroups_v2 {
    include!(concat!(env!("OUT_DIR"), "/io.containerd.cgroups.v2.rs"));
}

#[derive(Debug, Clone, Serialize)]
pub struct ContainerStats {
    pub id: String,
    pub name: String,
    pub cpu_usage: f64,
    pub cpu_usage_usec: u64,
    pub cpu_user_usec: u64,
    pub cpu_sys_usec: u64,
    pub cpu_nr_periods: u64,
    pub cpu_nr_throttled: u64,
    pub cpu_throttled_usec: u64,
    pub mem_usage: u64,
    pub mem_limit: u64,
    pub mem_swap_usage: u64,
    pub mem_swap_limit: u64,
    pub mem_pgfault: u64,
    pub mem_pgmajfault: u64,
    pub net_in: u64,
    pub net_out: u64,
    pub net_in_bps: f64,
    pub net_out_bps: f64,
    pub net_in_packets: u64,
    pub net_out_packets: u64,
    pub blk_in: u64,
    pub blk_out: u64,
    pub blk_in_byteps: f64,
    pub blk_out_byteps: f64,
    pub blk_in_ios: u64,
    pub blk_out_ios: u64,
    pub pids: u64,
}
impl Default for ContainerStats {
    fn default() -> Self {
        Self {
            id: Default::default(),
            name: Default::default(),
            cpu_usage: Default::default(),
            cpu_usage_usec: Default::default(),
            cpu_user_usec: Default::default(),
            cpu_sys_usec: Default::default(),
            cpu_nr_periods: Default::default(),
            cpu_nr_throttled: Default::default(),
            cpu_throttled_usec: Default::default(),
            mem_usage: Default::default(),
            mem_limit: Default::default(),
            mem_swap_usage: Default::default(),
            mem_swap_limit: Default::default(),
            mem_pgfault: Default::default(),
            mem_pgmajfault: Default::default(),
            net_in: Default::default(),
            net_out: Default::default(),
            net_in_bps: Default::default(),
            net_out_bps: Default::default(),
            net_in_packets: Default::default(),
            net_out_packets: Default::default(),
            blk_in: Default::default(),
            blk_out: Default::default(),
            blk_in_ios: Default::default(),
            blk_out_ios: Default::default(),
            blk_in_byteps: Default::default(),
            blk_out_byteps: Default::default(),
            pids: Default::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TimedContainerStatsResponse {
    id: String,
    name: String,
    stat: Option<ContainerStatsResponse>,
    time: SystemTime,
}

#[derive(Debug, Clone)]
struct LastDockerAPIContainersStats {
    pub timestamp: SystemTime,
    pub stats: HashMap<String, TimedContainerStatsResponse>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LastDockerStats {
    pub timestamp: SystemTime,
    pub stats: Vec<ContainerStats>,
}

#[derive(Debug)]
pub struct DockerStatPollingWorker {
    runtime: String,
    runtime_proc: String,
    namespace: String,
    host: String,
    prom_registry_prefix: String,
    delay_ms: Arc<Mutex<u64>>,

    containerd_channel: OnceLock<ContainerdChannel>,

    /// last collected docker stats record
    last_stats: Arc<Mutex<LastDockerStats>>,

    /// last records of `GET /container/{id}/stats` api
    last_docker_stats: Arc<Mutex<LastDockerAPIContainersStats>>,
}

/// raspberry pi did not have precpu_stats data, we need to get CPU usage by hand
/// reference at https://docs.docker.com/reference/api/engine/version/v1.52/#tag/Container/operation/ContainerStats
/// unit in ratio, not percent
fn get_cpu_usage(first: &ContainerCpuStats, second: &ContainerCpuStats, time_delta: f64) -> f64 {
    let cpu_delta = if let (Some(first), Some(second)) = (&first.cpu_usage, &second.cpu_usage) {
        if let (Some(first_total_usage), Some(second_total_usage)) =
            (first.total_usage, second.total_usage)
        {
            second_total_usage.saturating_sub(first_total_usage)
        } else {
            0
        }
    } else {
        0
    };

    let system_cpu_delta = if let (Some(first), Some(second)) =
        ((first.system_cpu_usage), (second.system_cpu_usage))
    {
        second - first
    } else {
        0
    };

    let online_cpus = if let Some(u) = second.online_cpus {
        u
    } else {
        0
    };

    let cpu_delta = cpu_delta as f64;
    let system_cpu_delta = system_cpu_delta as f64;
    let online_cpus = online_cpus as f64;

    let usage = (cpu_delta / system_cpu_delta) * online_cpus as f64 * time_delta;
    if usage > online_cpus {
        online_cpus
    } else {
        usage
    }
}

fn get_net_io(networks: &HashMap<String, ContainerNetworkStats>) -> (u64, u64, u64, u64) {
    let mut net_in = 0;
    let mut net_out = 0;
    let mut pkt_in = 0;
    let mut pkt_out = 0;

    for (_, net) in networks {
        net_in += net.rx_bytes.unwrap_or(0);
        net_out += net.tx_bytes.unwrap_or(0);
        pkt_in += net.rx_packets.unwrap_or(0);
        pkt_out += net.tx_packets.unwrap_or(0);
    }

    return (net_in, net_out, pkt_in, pkt_out);
}

fn get_blk_io(blk_stat: &ContainerBlkioStats) -> (u64, u64) {
    let mut blk_in = 0;
    let mut blk_out = 0;

    if let Some(v) = &blk_stat.io_service_bytes_recursive {
        for blk in v {
            let op = blk.op.as_deref();
            if op == Some("read") {
                if let Some(value) = blk.value {
                    blk_in += value
                }
            } else if op == Some("write") {
                if let Some(value) = blk.value {
                    blk_out += value
                }
            }
        }
    }

    return (blk_in, blk_out);
}

async fn docker_stat_oneshot(host: &str) -> Result<Vec<TimedContainerStatsResponse>, io::Error> {
    let docker = if host == "unix:///var/run/docker.sock" {
        match Docker::connect_with_defaults() {
            Ok(d) => d,
            Err(e) => return Err(io::Error::new(io::ErrorKind::BrokenPipe, e)),
        }
    } else {
        match host.parse::<Uri>() {
            Ok(u) => {
                let docker_result = match u.scheme_str() {
                    Some("http") => Docker::connect_with_http(host, 4, API_DEFAULT_VERSION),
                    // Some("https") => {
                    //     let _ = rustls::crypto::CryptoProvider::install_default(aws_lc_rs::default_provider());
                    //     let uri_parts = u.into_parts();
                    //     let addr = format!("tcp://{}{}",
                    //         uri_parts.authority.map(|a| a.to_string()).unwrap_or("".to_owned()),
                    //         uri_parts.path_and_query.map(|pq| pq.to_string()).unwrap_or("".to_owned()));
                    //     Docker::connect_with_ssl(&addr, Path::new("./key.pem"), Path::new("./cert.pem"), Path::new("./ca.pem"), 4, API_DEFAULT_VERSION)
                    //     Docker::connect_with_unix(path, timeout, client_version)
                    // },
                    _ => {
                        warn!("not supported docker uri scheme, fallback to defaults");
                        Docker::connect_with_defaults()
                    }
                };

                match docker_result {
                    Ok(d) => d,
                    Err(e) => return Err(io::Error::new(io::ErrorKind::BrokenPipe, e)),
                }
            }
            Err(_) => {
                warn!("invalid docker uri, fallback to defaults");
                match Docker::connect_with_defaults() {
                    Ok(d) => d,
                    Err(e) => return Err(io::Error::new(io::ErrorKind::BrokenPipe, e)),
                }
            }
        }
    };

    let mut filters = HashMap::new();
    filters.insert(
        "status".to_owned(),
        vec!["running".to_owned(), "paused".to_owned()],
    );

    let list_containers_options = Some(
        ListContainersOptionsBuilder::new()
            .all(true)
            .filters(&filters)
            .build(),
    );

    let start_at = SystemTime::now();
    let containers = match docker.list_containers(list_containers_options).await {
        Ok(v) => v,
        Err(e) => return Err(io::Error::new(io::ErrorKind::BrokenPipe, e)),
    };
    debug!(
        "containers listed from api in {} μs",
        SystemTime::now()
            .duration_since(start_at)
            .unwrap()
            .as_micros()
    );

    let mut stats: Vec<TimedContainerStatsResponse> = Vec::new();

    let start_at = SystemTime::now();
    for container in containers.iter() {
        let id = if let Some(s) = &container.id {
            s
        } else {
            continue;
        };
        let name = if let Some(v) = &container.names {
            if let Some(s) = v.first() {
                s
            } else {
                continue;
            }
        } else {
            continue;
        };

        let stats_option = Some(
            StatsOptionsBuilder::new()
                .stream(false)
                .one_shot(true)
                .build(),
        );
        let stats_stream = docker.stats(&id, stats_option);
        match stats_stream.try_collect::<Vec<_>>().await {
            Ok(v) => {
                let time = SystemTime::now();
                stats.push(TimedContainerStatsResponse {
                    id: id.clone(),
                    name: name.clone(),
                    stat: v.first().map(|e| e.clone()),
                    time: time,
                });
            }
            Err(e) => {
                error!("stats error: {}", e);
            }
        };
    }
    debug!(
        "stats of all containers from api in {} μs",
        SystemTime::now()
            .duration_since(start_at)
            .unwrap()
            .as_micros()
    );

    Ok(stats)
}

async fn podman_stat_oneshot(host: &str) -> Result<Vec<TimedContainerStatsResponse>, io::Error> {
    todo!()
}

impl DockerStatPollingWorker {
    async fn containerd_stat_oneshot(&self, host: &str) -> Result<Vec<ContainerStats>, io::Error> {
        let channel = match self.containerd_channel.get() {
            Some(c) => c,
            None => {
                let host = match host.strip_prefix("unix://") {
                    Some(h) => h,
                    None => {
                        error!("containerd_stat_oneshot: only supports unix socket");
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "only supports unix socket in containerd runtime",
                        ));
                    }
                };
                let channel = match containerd_client::connect(host).await {
                    Ok(channel) => channel,
                    Err(e) => {
                        error!("containerd_client::connect failed, error: {}", e);
                        return Err(io::Error::new(io::ErrorKind::BrokenPipe, e));
                    }
                };

                self.containerd_channel.set(channel.clone()).unwrap();
                &channel.clone()
            }
        };

        let mut id_name_map = HashMap::new();
        let mut id_pid_map = HashMap::new();

        let mut client = containerd_client::services::v1::containers_client::ContainersClient::new(
            channel.clone(),
        );
        let request = ListContainersRequest::default();
        let request = with_namespace!(request, &self.namespace);
        match client.list(request).await {
            Ok(r) => {
                let response = r.get_ref().clone();
                for container in response.containers {
                    if let Some(name) = container.labels.get("nerdctl/name") {
                        id_name_map.insert(container.id.clone(), name.clone());
                    }
                }
            }
            Err(e) => {
                error!("containerd_client::list failed, error: {}", e);
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, e));
            }
        };

        let mut client =
            containerd_client::services::v1::tasks_client::TasksClient::new(channel.clone());

        let request = ListTasksRequest::default();
        let request = with_namespace!(request, &self.namespace);
        match client.list(request).await {
            Ok(r) => {
                let response = r.get_ref();
                for task in response.tasks.clone() {
                    trace!("task: {:?}", task);
                    id_pid_map.insert(task.id.clone(), task.pid);
                }
            }
            Err(e) => {
                error!("containerd_client::list failed, error: {}", e);
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, e));
            }
        }

        let request = MetricsRequest::default();
        let request = with_namespace!(request, self.namespace);

        let response = match client.metrics(request).await {
            Ok(r) => r.get_ref().clone(),
            Err(e) => {
                error!("containerd_client::metrics failed, error: {}", e);
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, e));
            }
        };
        let mut containerd_metrics = Vec::new();
        for metric in response.metrics {
            trace!("metric: {:?}", metric);
            let Some(data) = metric.data.as_ref() else {
                warn!("metric '{}' has no data", metric.id);
                continue;
            };
            let metric_data = match data.type_url.as_str() {
                "io.containerd.cgroups.v2.Metrics" => {
                    match cgroups_v2::Metrics::decode(data.value.as_slice()) {
                        Ok(decoded) => decoded,
                        Err(e) => {
                            warn!("failed to decode containerd metrics: {}", e);
                            continue;
                        }
                    }
                }
                other => {
                    warn!("unsupported containerd metric type: {}", other);
                    continue;
                }
            };
            trace!("metric_data: {:?}", metric_data);

            let name = id_name_map.get(&metric.id).unwrap_or(&metric.id);
            let mut stats = ContainerStats {
                id: metric.id.clone(),
                name: name.clone(),
                ..Default::default()
            };
            trace!(
                "container {}, pid: {:?}, id: {}",
                name,
                id_pid_map.get(&metric.id),
                metric.id
            );

            if let Some(cpu_stat) = metric_data.cpu {
                trace!(
                    "  cpu usage_usec={} user_usec={} system_usec={}",
                    cpu_stat.usage_usec, cpu_stat.user_usec, cpu_stat.system_usec
                );
                stats.cpu_usage_usec = cpu_stat.usage_usec;
                stats.cpu_user_usec = cpu_stat.user_usec;
                stats.cpu_sys_usec = cpu_stat.system_usec;
                stats.cpu_nr_periods = cpu_stat.nr_periods;
                stats.cpu_nr_throttled = cpu_stat.nr_throttled;
                stats.cpu_throttled_usec = cpu_stat.throttled_usec;
            }
            if let Some(mem_stat) = metric_data.memory {
                trace!(
                    "  mem usage={} limit={} swap_usage={} swap_limit={} pgfault={} pgmajfault={}",
                    mem_stat.usage,
                    mem_stat.usage_limit,
                    mem_stat.swap_usage,
                    mem_stat.swap_limit,
                    mem_stat.pgfault,
                    mem_stat.pgmajfault
                );
                stats.mem_usage = mem_stat.usage.saturating_sub(mem_stat.file);
                stats.mem_limit = if mem_stat.usage_limit == u64::MAX {
                    0
                } else {
                    mem_stat.usage_limit
                };
                stats.mem_swap_usage = mem_stat.swap_usage;
                stats.mem_swap_limit = if mem_stat.swap_limit == u64::MAX {
                    0
                } else {
                    mem_stat.swap_limit
                };
                stats.mem_pgfault = mem_stat.pgfault;
                stats.mem_pgmajfault = mem_stat.pgmajfault;
            }
            if let Some(io_stat) = metric_data.io {
                let mut io_sum = IoEntry::default();
                for entry in io_stat.usage {
                    io_sum.major += entry.major;
                    io_sum.minor += entry.minor;
                    io_sum.rbytes += entry.rbytes;
                    io_sum.wbytes += entry.wbytes;
                    io_sum.rios += entry.rios;
                    io_sum.wios += entry.wios;
                }
                trace!(
                    "  io rbytes: {}, wbytes: {}, rios: {}, wios: {}",
                    io_sum.rbytes, io_sum.wbytes, io_sum.rios, io_sum.wios
                );

                stats.blk_in = io_sum.rbytes;
                stats.blk_out = io_sum.wbytes;
                stats.blk_in_ios = io_sum.rios;
                stats.blk_out_ios = io_sum.wios;
            }

            if let Some(pid) = id_pid_map.get(&metric.id) {
                let netdev_path = format!("{}/{}/net/dev", self.runtime_proc, pid);
                match std::fs::read_to_string(&netdev_path) {
                    Ok(content) => {
                        let mut network_stat = NetworkStat::default();

                        for line in content.lines().skip(2) {
                            let Some((iface, rest)) = line.split_once(':') else {
                                continue;
                            };

                            if iface == "lo" {
                                continue;
                            }

                            let iface = iface.trim();
                            let fields: Vec<&str> = rest.split_whitespace().collect();
                            let rx_bytes: u64 = fields[0].parse().unwrap_or(0);
                            let rx_packets: u64 = fields[1].parse().unwrap_or(0);
                            let rx_errs: u64 = fields[2].parse().unwrap_or(0);
                            let rx_dropped: u64 = fields[3].parse().unwrap_or(0);
                            let tx_bytes: u64 = fields[8].parse().unwrap_or(0);
                            let tx_packets: u64 = fields[9].parse().unwrap_or(0);
                            let tx_errs: u64 = fields[10].parse().unwrap_or(0);
                            let tx_dropped: u64 = fields[11].parse().unwrap_or(0);
                            trace!(
                                "  iface: {}, rx_bytes={} tx_bytes={} rx_packets={} tx_packets={} rx_errs={} tx_errs={} rx_dropped={} tx_dropped={}",
                                iface,
                                rx_bytes,
                                tx_bytes,
                                rx_packets,
                                tx_packets,
                                rx_errs,
                                tx_errs,
                                rx_dropped,
                                tx_dropped
                            );

                            network_stat.rx_bytes = network_stat.rx_bytes.saturating_add(rx_bytes);
                            network_stat.rx_packets =
                                network_stat.rx_packets.saturating_add(rx_packets);
                            network_stat.rx_errors = network_stat.rx_errors.saturating_add(rx_errs);
                            network_stat.rx_dropped =
                                network_stat.rx_dropped.saturating_add(rx_dropped);
                            network_stat.tx_bytes = network_stat.tx_bytes.saturating_add(tx_bytes);
                            network_stat.tx_packets =
                                network_stat.tx_packets.saturating_add(tx_packets);
                            network_stat.tx_errors = network_stat.tx_errors.saturating_add(tx_errs);
                            network_stat.tx_dropped =
                                network_stat.tx_dropped.saturating_add(tx_dropped);
                        }

                        stats.net_in = network_stat.rx_bytes;
                        stats.net_out = network_stat.tx_bytes;
                        stats.net_in_packets = network_stat.rx_packets;
                        stats.net_out_packets = network_stat.tx_packets;
                    }
                    Err(e) => {
                        warn!(
                            "retreive network metrics data failed, path: {}, error: {:?}",
                            netdev_path, e
                        );
                    }
                }
            }

            if let Some(pid_stat) = metric_data.pids {
                stats.pids = pid_stat.current;
            }

            containerd_metrics.push(stats);
        }

        Ok(containerd_metrics)
    }

    pub async fn task_handler(&self) {
        trace!("runtime: {}, host: {}", self.runtime, self.host);

        loop {
            // get last docker stats from api
            let last_api_stats = match self.runtime.as_str() {
                "docker" => docker_stat_oneshot(&self.host).await,
                "containerd" => {
                    let mut metrics = match self.containerd_stat_oneshot(&self.host).await {
                        Ok(v) => v,
                        Err(e) => {
                            error!("containerd_stat_oneshot failed, error: {}", e);

                            let delay = {
                                let delay_guard = self.delay_ms.lock().await;
                                Duration::from_millis(*delay_guard)
                            };
                            tokio::time::sleep(delay).await;

                            continue;
                        }
                    };

                    let _ = {
                        let mut last_stat_guard = self.last_stats.lock().await;
                        last_stat_guard.timestamp = SystemTime::now();
                        last_stat_guard.stats.clear();
                        last_stat_guard.stats.append(&mut metrics);
                    };

                    let delay = {
                        let delay_guard = self.delay_ms.lock().await;
                        Duration::from_millis(*delay_guard)
                    };
                    tokio::time::sleep(delay).await;

                    continue;
                }
                "podman" => podman_stat_oneshot(&self.host).await,
                _ => {
                    error!("runtime {} not supported", self.runtime);
                    break;
                }
            };
            let last_api_stats = match last_api_stats {
                Ok(v) => v,
                Err(e) => {
                    error!("docker_stat_oneshot failed, error: {}", e);

                    let delay = {
                        let delay_guard = self.delay_ms.lock().await;
                        Duration::from_millis(*delay_guard)
                    };
                    tokio::time::sleep(delay).await;
                    continue;
                }
            };
            let whole_start_at = SystemTime::now();

            let mut parsed_stat = Vec::new();

            let start_at = SystemTime::now();
            for container_api_stat in last_api_stats.iter() {
                let mut stat = ContainerStats {
                    id: container_api_stat.id.clone(),
                    name: container_api_stat.name.clone(),
                    ..Default::default()
                };

                if let Some(ref s) = container_api_stat.stat {
                    if let Some(cpu_stats) = &s.cpu_stats {
                        if let Some(cpu_usage) = &cpu_stats.cpu_usage {
                            // usage data is in nanoseconds
                            if let Some(ns) = cpu_usage.total_usage {
                                stat.cpu_usage_usec = ns / 1_000;
                            }
                            if let Some(ns) = cpu_usage.usage_in_kernelmode {
                                stat.cpu_sys_usec = ns / 1_000;
                            }
                            if let Some(ns) = cpu_usage.usage_in_usermode {
                                stat.cpu_user_usec = ns / 1_000;
                            }
                        }
                        if let Some(t) = &cpu_stats.throttling_data {
                            if let Some(p) = t.periods {
                                stat.cpu_nr_periods = p;
                            }
                            if let Some(p) = t.throttled_time {
                                stat.cpu_nr_throttled = p;
                            }
                            if let Some(ns) = t.throttled_periods {
                                stat.cpu_throttled_usec = ns / 1_000;
                            }
                        }

                        let system_cpu_usage = cpu_stats.system_cpu_usage.unwrap_or(0) as f64;
                        let total_usage = if let Some(u) = &cpu_stats.cpu_usage {
                            u.total_usage.unwrap_or(0) as f64
                        } else {
                            0.
                        };

                        stat.cpu_usage = total_usage / system_cpu_usage;
                    }

                    if let Some(mem_stats) = &s.memory_stats {
                        if let Some(limit) = mem_stats.limit {
                            stat.mem_limit = limit;
                        }
                        if let (Some(usage), Some(stats)) = (mem_stats.usage, &mem_stats.stats) {
                            if let Some(file) = stats.get("file") {
                                stat.mem_usage = usage.saturating_sub(*file);
                            }
                        }
                    }

                    if let Some(networks) = &s.networks {
                        let (net_in, net_out, pkt_in, pkt_out) = get_net_io(networks);
                        stat.net_in = net_in;
                        stat.net_out = net_out;
                        stat.net_in_packets = pkt_in;
                        stat.net_out_packets = pkt_out;
                    }

                    if let Some(blkio_stats) = &s.blkio_stats {
                        let (blk_in, blk_out) = get_blk_io(blkio_stats);
                        stat.blk_in = blk_in;
                        stat.blk_out = blk_out;
                    }

                    if let Some(pids) = &s.pids_stats {
                        if let Some(pids) = pids.current {
                            stat.pids = pids;
                        }
                    }
                }

                // previous docker stat from api
                let pre_api_stat = {
                    let stat_guard = self.last_docker_stats.lock().await;
                    stat_guard
                        .stats
                        .get(&container_api_stat.id)
                        .map(|s| s.clone())
                };

                if let Some(pre_api_stat) = pre_api_stat {
                    if let (Some(pre_container_stat), Some(container_stat)) =
                        (pre_api_stat.stat, &container_api_stat.stat)
                    {
                        let duration = container_api_stat
                            .time
                            .duration_since(pre_api_stat.time)
                            .unwrap();
                        let time_delta = 1_000_000_000. / duration.as_nanos() as f64;

                        // get cpu use between the stats
                        let cpu_usage = if let (Some(first_cpustat), Some(second_cpu_stat)) =
                            (&pre_container_stat.cpu_stats, &container_stat.cpu_stats)
                        {
                            get_cpu_usage(first_cpustat, second_cpu_stat, time_delta)
                        } else {
                            0.0
                        };
                        stat.cpu_usage = cpu_usage;

                        // get netio bps between the stats
                        let (first_net_in, first_net_out, first_pkt_in, first_pkt_out) =
                            if let Some(networks) = &pre_container_stat.networks {
                                get_net_io(networks)
                            } else {
                                (0, 0, 0, 0)
                            };
                        let (net_in_bps, net_out_bps) = (
                            if stat.net_in > first_net_in {
                                (stat.net_in.saturating_sub(first_net_in)) as f64 * time_delta
                            } else {
                                0.0
                            },
                            if stat.net_out > first_net_out {
                                (stat.net_out.saturating_sub(first_net_out)) as f64 * time_delta
                            } else {
                                0.0
                            },
                        );
                        stat.net_in_bps = net_in_bps * 8.;
                        stat.net_out_bps = net_out_bps * 8.;
                        stat.net_in_packets = first_pkt_in;
                        stat.net_out_packets = first_pkt_out;

                        // get blkio bps between the stats
                        let (first_blk_in, first_blk_out) =
                            if let Some(blkio) = &pre_container_stat.blkio_stats {
                                get_blk_io(blkio)
                            } else {
                                (0, 0)
                            };
                        let (blk_in_byteps, blk_out_byteps) = (
                            if stat.blk_in > first_blk_in {
                                (stat.blk_in.saturating_sub(first_blk_in)) as f64 * time_delta
                            } else {
                                0.0
                            },
                            if stat.blk_out > first_blk_out {
                                (stat.blk_out.saturating_sub(first_blk_out)) as f64 * time_delta
                            } else {
                                0.0
                            },
                        );
                        stat.blk_in_byteps = blk_in_byteps;
                        stat.blk_out_byteps = blk_out_byteps;
                    }
                }

                parsed_stat.push(stat);
            }
            debug!(
                "parsed all containers stats in {} μs",
                SystemTime::now()
                    .duration_since(start_at)
                    .unwrap()
                    .as_micros() as u64
            );

            // update last status for next probe
            let _ = {
                let mut last_stat_guard = self.last_stats.lock().await;
                last_stat_guard.timestamp = whole_start_at;
                last_stat_guard.stats.clear();
                last_stat_guard.stats.append(&mut parsed_stat);
            };

            let _ = {
                let mut last_api_stat_guard = self.last_docker_stats.lock().await;
                last_api_stat_guard.timestamp = whole_start_at;
                last_api_stat_guard.stats.clear();
                for api_stat in last_api_stats {
                    last_api_stat_guard
                        .stats
                        .insert(api_stat.id.clone(), api_stat);
                }
            };

            let delay = {
                let delay_guard = self.delay_ms.lock().await;
                Duration::from_millis(*delay_guard)
            };
            tokio::time::sleep(delay).await;
            // self.print_stat().await;
        }
    }

    pub fn new(
        runtime: &str,
        runtime_proc: &str,
        namespace: &str,
        host: &str,
        polling_millis: u64,
    ) -> Self {
        Self {
            runtime: runtime.to_owned(),
            runtime_proc: runtime_proc.to_owned(),
            namespace: namespace.to_owned(),
            host: host.to_owned(),
            prom_registry_prefix: "container".to_string(),
            delay_ms: Arc::new(Mutex::new(polling_millis)),
            containerd_channel: OnceLock::new(),
            last_stats: Arc::new(Mutex::new(LastDockerStats {
                timestamp: SystemTime::now(),
                stats: Vec::new(),
            })),
            last_docker_stats: Arc::new(Mutex::new(LastDockerAPIContainersStats {
                timestamp: SystemTime::now(),
                stats: HashMap::new(),
            })),
        }
    }

    pub async fn get_cgroup2_data(
        &self,
        id: &str,
    ) -> Result<TimedContainerStatsResponse, io::Error> {
        let stats = {
            let stats_guard = self.last_docker_stats.lock().await;
            let container_stat = stats_guard.stats.get(id);
            container_stat.map(|s| s.clone())
        };

        match stats {
            Some(s) => Ok(s),
            None => Err(io::Error::new(io::ErrorKind::InvalidInput, "id not found")),
        }
    }

    pub async fn get_last_container_stats(&self) -> LastDockerStats {
        self.last_stats.lock().await.clone()
    }

    pub async fn get_last_container_stats_registry(&self) -> Registry {
        let mut registry = Registry::with_prefix(self.prom_registry_prefix.clone());

        let _ = {
            let stat_guard = self.last_stats.lock().await;
            for stat in stat_guard.stats.iter() {
                let metrics =
                    DockerStatContainerMetrics::new(&self.runtime, &self.namespace, &stat.id);
                metrics.cpu_usage.set(stat.cpu_usage);
                metrics
                    .cpu_usage_acc
                    .set(stat.cpu_usage_usec as f64 / 1_000_000.0);
                metrics
                    .cpu_user_acc
                    .set(stat.cpu_user_usec as f64 / 1_000_000.0);
                metrics
                    .cpu_system_acc
                    .set(stat.cpu_sys_usec as f64 / 1_000_000.0);
                metrics.cpu_nr_periods.set(stat.cpu_nr_periods);
                metrics.cpu_nr_throttled.set(stat.cpu_nr_throttled);
                metrics
                    .cpu_throttled_acc
                    .set(stat.cpu_throttled_usec as f64 / 1_000_000.0);
                metrics.mem_usage.set(stat.mem_usage);
                metrics.mem_limit.set(stat.mem_limit);
                metrics.mem_swap_usage.set(stat.mem_swap_usage);
                metrics.mem_swap_limit.set(stat.mem_swap_limit);
                metrics.mem_pgfault.set(stat.mem_pgfault);
                metrics.mem_pgmajfault.set(stat.mem_pgmajfault);
                metrics.net_in.set(stat.net_in);
                metrics.net_out.set(stat.net_out);
                metrics.net_in_bps.set(stat.net_in_bps);
                metrics.net_out_bps.set(stat.net_out_bps);
                metrics.net_in_packets.set(stat.net_in_packets);
                metrics.net_out_packets.set(stat.net_out_packets);
                metrics.blk_in.set(stat.blk_in);
                metrics.blk_out.set(stat.blk_out);
                metrics.blk_in_byteps.set(stat.blk_in_byteps);
                metrics.blk_out_byteps.set(stat.blk_out_byteps);
                metrics.blk_in_ios.set(stat.blk_in_ios);
                metrics.blk_out_ios.set(stat.blk_out_ios);
                metrics.pids.set(stat.pids);

                let name = stat.name.strip_prefix("/").unwrap_or(stat.name.as_str());
                metrics.register_as_sub_registry(&mut registry, name);
            }
        };
        registry
    }

    pub fn set_delay(&self, duration: Duration) {
        let mut delay = self.delay_ms.blocking_lock();
        *delay = duration.as_millis() as u64;
    }

    pub async fn print_stat(&self) {
        let last_stats_guard = self.last_stats.lock().await;
        println!("Last probe at {:?}", last_stats_guard.timestamp);
        println!("stats:");
        println!("");
        for stat in last_stats_guard.stats.iter() {
            let formatted_line = format!(
                "{} {} {:.4} {} {} {} {} {}",
                &stat.id[..7],
                &stat.name[1..],
                stat.cpu_usage,
                stat.mem_usage,
                stat.net_in,
                stat.net_out,
                stat.blk_in,
                stat.blk_out
            );
            println!("{}", formatted_line);
        }
    }
}
