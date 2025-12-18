use std::{
    collections::HashMap,
    io,
    sync::Arc,
    time::{Duration, SystemTime},
};

use actix_web::http::Uri;
use bollard::{
    API_DEFAULT_VERSION, Docker,
    query_parameters::{ListContainersOptionsBuilder, StatsOptionsBuilder},
    secret::{
        ContainerBlkioStats, ContainerCpuStats, ContainerMemoryStats, ContainerNetworkStats,
        ContainerStatsResponse,
    },
};
use futures_util::TryStreamExt;
use prometheus_client::registry::Registry;
use serde::Serialize;
use tokio::{sync::Mutex, task::JoinHandle};
use tracing::*;

use crate::docker_stat_metrics::DockerStatContainerMetrics;

#[derive(Debug, Clone, Serialize)]
pub struct DockerContainerStat {
    pub id: String,
    pub name: String,
    pub cpu_usage: f64,
    pub mem_usage: u64,
    pub mem_limit: u64,
    pub net_in: u64,
    pub net_out: u64,
    pub net_in_bps: f64,
    pub net_out_bps: f64,
    pub blk_in: u64,
    pub blk_out: u64,
    pub blk_in_byteps: f64,
    pub blk_out_byteps: f64,
}
impl Default for DockerContainerStat {
    fn default() -> Self {
        Self {
            id: Default::default(),
            name: Default::default(),
            cpu_usage: Default::default(),
            mem_usage: Default::default(),
            mem_limit: Default::default(),
            net_in: Default::default(),
            net_out: Default::default(),
            net_in_bps: Default::default(),
            net_out_bps: Default::default(),
            blk_in: Default::default(),
            blk_out: Default::default(),
            blk_in_byteps: Default::default(),
            blk_out_byteps: Default::default(),
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

/// raspberry pi did not have precpu_stats data, we need to get CPU usage by hand
/// reference at https://docs.docker.com/reference/api/engine/version/v1.52/#tag/Container/operation/ContainerStats
/// unit in ratio, not percent
fn get_cpu_usage(first: &ContainerCpuStats, second: &ContainerCpuStats, time_delta: f64) -> f64 {
    let cpu_delta = if let (Some(first), Some(second)) = (&first.cpu_usage, &second.cpu_usage) {
        if let (Some(first_total_usage), Some(second_total_usage)) =
            (first.total_usage, second.total_usage)
        {
            second_total_usage - first_total_usage
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

    (cpu_delta / system_cpu_delta) * online_cpus as f64 * time_delta
}

fn get_mem(mem: &ContainerMemoryStats) -> Result<u64, io::Error> {
    let usage = if let Some(u) = mem.usage {
        u
    } else {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "no usage"));
    };

    if let Some(stats) = &mem.stats {
        if let Some(file) = stats.get("file") {
            return Ok(usage - file);
        }

        return Err(io::Error::new(io::ErrorKind::InvalidInput, "no file"));
    }

    return Err(io::Error::new(io::ErrorKind::InvalidInput, "no stat"));
}

fn get_net_io(networks: &HashMap<String, ContainerNetworkStats>) -> (u64, u64) {
    let mut net_in = 0;
    let mut net_out = 0;

    for (_, net) in networks {
        net_in += net.rx_bytes.unwrap_or(0);
        net_out += net.tx_bytes.unwrap_or(0);
    }

    return (net_in, net_out);
}

fn get_blk_io(networks: &ContainerBlkioStats) -> (u64, u64) {
    let mut net_in = 0;
    let mut net_out = 0;

    if let Some(v) = &networks.io_service_bytes_recursive {
        for blk in v {
            let op = blk.op.as_deref();
            if op == Some("read") {
                if let Some(value) = blk.value {
                    net_in += value
                }
            } else if op == Some("write") {
                if let Some(value) = blk.value {
                    net_out += value
                }
            }
        }
    }

    return (net_in, net_out);
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

#[derive(Debug, Clone)]
struct LastDockerAPIContainersStats {
    pub timestamp: SystemTime,
    pub stats: HashMap<String, TimedContainerStatsResponse>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LastDockerStats {
    pub timestamp: SystemTime,
    pub stats: Vec<DockerContainerStat>,
}

#[derive(Debug)]
pub struct DockerStatPollingWorker {
    docker_host: String,
    prom_registry_prefix: Arc<Mutex<String>>,
    delay_ms: Arc<Mutex<u64>>,

    /// last collected docker stats record
    last_stats: Arc<Mutex<LastDockerStats>>,

    /// last records of `GET /container/{id}/stats` api
    last_docker_stats: Arc<Mutex<LastDockerAPIContainersStats>>,
}

impl DockerStatPollingWorker {
    async fn task_handler(&self) {
        loop {
            // get last docker stats from api
            let last_api_stats = match docker_stat_oneshot(&self.docker_host).await {
                Ok(v) => v,
                Err(e) => {
                    error!("docker_stat_oneshot failed, error: {}", e);
                    continue;
                }
            };
            let whole_start_at = SystemTime::now();

            let mut parsed_stat = Vec::new();

            let start_at = SystemTime::now();
            for container_api_stat in last_api_stats.iter() {
                let mut stat = if let Some(ref s) = container_api_stat.stat {
                    let cpu_usage = if let Some(cpu_stats) = &s.cpu_stats {
                        let system_cpu_usage = cpu_stats.system_cpu_usage.unwrap_or(0) as f64;
                        let total_usage = if let Some(u) = &cpu_stats.cpu_usage {
                            u.total_usage.unwrap_or(0) as f64
                        } else {
                            0.
                        };
                        total_usage / system_cpu_usage
                    } else {
                        0.
                    };

                    let (mem_usage, mem_limit) = if let Some(mem_stats) = &s.memory_stats {
                        let limit = mem_stats.limit.unwrap_or(0);
                        let usage = match get_mem(&mem_stats) {
                            Ok(u) => u,
                            Err(e) => {
                                warn!("get_mem failed, error: {}", e);
                                0
                            }
                        };
                        (usage, limit)
                    } else {
                        (0, 0)
                    };

                    // net io
                    let (net_in, net_out) = if let Some(networks) = &s.networks {
                        get_net_io(networks)
                    } else {
                        (0, 0)
                    };

                    // blk io
                    let (blk_in, blk_out) = if let Some(blkio) = &s.blkio_stats {
                        get_blk_io(blkio)
                    } else {
                        (0, 0)
                    };

                    DockerContainerStat {
                        id: container_api_stat.id.clone(),
                        name: container_api_stat.name.clone(),
                        cpu_usage,
                        mem_usage,
                        mem_limit,
                        net_in,
                        net_out,
                        blk_in,
                        blk_out,
                        ..Default::default()
                    }
                } else {
                    DockerContainerStat {
                        id: container_api_stat.id.clone(),
                        name: container_api_stat.name.clone(),
                        ..Default::default()
                    }
                };

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
                        let (first_net_in, first_net_out) =
                            if let Some(networks) = &pre_container_stat.networks {
                                get_net_io(networks)
                            } else {
                                (0, 0)
                            };
                        let (net_in_bps, net_out_bps) = (
                            (stat.net_in - first_net_in) as f64 * time_delta,
                            (stat.net_out - first_net_out) as f64 * time_delta,
                        );
                        stat.net_in_bps = net_in_bps * 8.;
                        stat.net_out_bps = net_out_bps * 8.;

                        // get blkio bps between the stats
                        let (first_blk_in, first_blk_out) =
                            if let Some(blkio) = &pre_container_stat.blkio_stats {
                                get_blk_io(blkio)
                            } else {
                                (0, 0)
                            };
                        let (blk_in_byteps, blk_out_byteps) = (
                            (stat.blk_in - first_blk_in) as f64 * time_delta,
                            (stat.blk_out - first_blk_out) as f64 * time_delta,
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

    pub fn new(host: &str, polling_millis: u64) -> Self {
        Self {
            docker_host: host.to_owned(),
            prom_registry_prefix: Arc::new(Mutex::new("container".to_owned())),
            delay_ms: Arc::new(Mutex::new(polling_millis)),
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

    pub fn spawn_polling_stat_task(&self, myself: Arc<Self>) -> JoinHandle<()> {
        tokio::spawn(async move { myself.task_handler().await })
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
        let registry_prefix = {
            let prefix_guard = self.prom_registry_prefix.lock().await;
            &prefix_guard.clone()
        };
        let mut registry = Registry::with_prefix(registry_prefix);

        let _ = {
            let stat_guard = self.last_stats.lock().await;
            for stat in stat_guard.stats.iter() {
                let metrics = DockerStatContainerMetrics::new(&stat.id);
                metrics.cpu_usage.set(stat.cpu_usage);
                metrics.mem_usage.set(stat.mem_usage);
                metrics.mem_limit.set(stat.mem_limit);
                metrics.net_in.set(stat.net_in);
                metrics.net_out.set(stat.net_out);
                metrics.net_in_bps.set(stat.net_in_bps);
                metrics.net_out_bps.set(stat.net_out_bps);
                metrics.blk_in.set(stat.blk_in);
                metrics.blk_out.set(stat.blk_out);
                metrics.blk_in_byteps.set(stat.blk_in_byteps);
                metrics.blk_out_byteps.set(stat.blk_out_byteps);
                
                metrics.register_as_sub_registry(&mut registry, &stat.name[1..]);
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
