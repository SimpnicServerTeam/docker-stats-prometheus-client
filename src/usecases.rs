use std::{collections::HashMap, io, time::{Duration, SystemTime}};

use bollard::{Docker, query_parameters::{ListContainersOptionsBuilder, StatsOptionsBuilder}, secret::{ContainerBlkioStats, ContainerCpuStats, ContainerMemoryStats, ContainerNetworkStats, ContainerStatsResponse}};
use futures_util::TryStreamExt;
use serde::Serialize;
use tracing::*;

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

#[derive(Debug, Clone, Serialize)]
struct TimedContainerStatsResponse {
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
        if let (Some(first_total_usage), Some(second_total_usage)) = (first.total_usage, second.total_usage) {
            second_total_usage - first_total_usage
        } else { 0 }
    } else { 0 };

    let system_cpu_delta = if let (Some(first), Some(second)) = ((first.system_cpu_usage), (second.system_cpu_usage)) {
        second - first
    } else { 0 };

    let online_cpus = if let Some(u) = second.online_cpus {
        u
    } else { 0 };

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

pub async fn docker_stat(host: &str) -> Result<Vec<DockerContainerStat>, io::Error> {
    let docker = match Docker::connect_with_defaults() {
        Ok(d) => d,
        Err(e) => return Err(io::Error::new(io::ErrorKind::BrokenPipe, e)),
    };

    let mut filters = HashMap::new();
    filters.insert("status".to_owned(), vec!["running".to_owned(), "paused".to_owned()]);

    let list_containers_options = Some(ListContainersOptionsBuilder::new()
        .all(true)
        .filters(&filters).build());

    let start_at = SystemTime::now();
    let containers = match docker.list_containers(list_containers_options).await {
        Ok(v) => v,
        Err(e) => return Err(io::Error::new(io::ErrorKind::BrokenPipe, e)),
    };
    debug!("containers listed from api in {} μs", SystemTime::now().duration_since(start_at).unwrap().as_micros());

    let mut first_stats: HashMap<String, TimedContainerStatsResponse> = HashMap::new();
    let mut second_stats = HashMap::new();

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

        let stats_option = Some(StatsOptionsBuilder::new()
            .stream(false).one_shot(true).build());
        let stats_stream = docker.stats(&id, stats_option);
        match stats_stream.try_collect::<Vec<_>>().await {
            Ok(v) => {
                let time = SystemTime::now();
                first_stats.insert(id.clone(), TimedContainerStatsResponse {
                    id: id.clone(),
                    name: name.clone(),
                    stat: v.first().map(|e| e.clone()),
                    time: time,
                });
            },
            Err(e) => {
                error!("stats error: {}", e);
            },
        }
    }
    debug!("first stats of all containers from api in {} μs", SystemTime::now().duration_since(start_at).unwrap().as_micros());

    tokio::time::sleep(Duration::from_millis(500)).await;

    let start_at = SystemTime::now();
    for (id, first) in first_stats.iter() {
        let stats_option = Some(StatsOptionsBuilder::new()
            .stream(false).one_shot(true).build());
        let stats_stream = docker.stats(&id, stats_option);
        match stats_stream.try_collect::<Vec<_>>().await {
            Ok(v) => {
                let time = SystemTime::now();
                second_stats.insert(id.clone(), TimedContainerStatsResponse {
                    id: id.clone(),
                    name: first.name.clone(),
                    stat: v.first().map(|e| e.clone()),
                    time: time,
                });
            },
            Err(e) => {
                error!("stats error: {}", e);
            },
        }
    }
    debug!("second stats of all containers from api in {} μs", SystemTime::now().duration_since(start_at).unwrap().as_micros());

    let mut container_stats = Vec::new();

    // let (sender, receiver) = std::sync::mpsc::channel();
    // let keys = second_stats.keys().collect::<Vec<&String>>();
    // keys.into_par_iter().for_each_with(sender, |s, name| {
    //     if let (Some(first_stat), Some(second_stat)) = (first_stats.get(name), second_stats.get(name)) {
    //         if let (Some(first), Some(second)) = (&first_stat.stat, &second_stat.stat) {
    //             // cpu%
    //             let cpu_delta = second.cpu_stats.clone().unwrap().cpu_usage.unwrap().total_usage.unwrap() - first.cpu_stats.clone().unwrap().cpu_usage.unwrap().total_usage.unwrap();
    //             let system_cpu_delta = second.cpu_stats.clone().unwrap().system_cpu_usage.unwrap() - first.cpu_stats.clone().unwrap().system_cpu_usage.unwrap();

    //             let cpu_delta = cpu_delta as f64;
    //             let system_cpu_delta = system_cpu_delta as f64;

    //             let first_nanos = first_stat.time.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos();
    //             let second_nanos = second_stat.time.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos();
    //             let time_delta = 1_000_000_000. / (second_nanos - first_nanos) as f64;
    //             // println!("{} time_delta: {}", name, time_delta);

    //             let cpu_usage_percent = (cpu_delta / system_cpu_delta) * second.cpu_stats.clone().unwrap().online_cpus.unwrap() as f64 * time_delta * 100.;
    //             // println!("{} cpu_usage_percent: {}", name, cpu_usage_percent);

    //             // memory
    //             let (mem_usage, mem_limit) = if let Some(mem) = &second.memory_stats {
    //                 let limit = mem.limit.unwrap();
    //                 let usage = get_mem(mem).unwrap();
    //                 // println!("{} mem_use: {} / {}", name, usage, limit);
    //                 (usage, limit)
    //             } else { (0, 0) };

    //             // net io
    //             let (first_net_in, first_net_out) = get_net_io(&first.networks.clone().unwrap());
    //             let (second_net_in, second_net_out) = get_net_io(&second.networks.clone().unwrap());
    //             let (net_in_bps, net_out_bps) = ((second_net_in - first_net_in) as f64 * time_delta, (second_net_out - first_net_out) as f64 * time_delta);
    //             // println!("{} net io: {} / {}", name, second_net_in, second_net_out);
    //             // println!("{} net io-bps: {} / {}", name, net_in_bps, net_out_bps);

    //             // blk io
    //             let (first_blk_in, first_blk_out) = get_blk_io(&first.blkio_stats.clone().unwrap());
    //             let (second_blk_in, second_blk_out) = get_blk_io(&second.blkio_stats.clone().unwrap());
    //             let (blk_in_byteps, blk_out_byteps) = ((second_blk_in - first_blk_in) as f64 * time_delta, (second_blk_out - first_blk_out) as f64 * time_delta);
    //             // println!("{} blk io: {} / {}", name, second_blk_in, second_blk_out);
    //             // println!("{} blk io-bps: {} / {}", name, io_in_byteps, io_out_byteps);

    //             let _ = s.send(DockerContainerStat { 
    //                 id: first_stat.clone().stat.unwrap().id.unwrap(), 
    //                 name: name.to_owned(), 
    //                 cpu_percent: cpu_usage_percent, 
    //                 mem_usage, 
    //                 mem_limit, 
    //                 net_in: second_net_in, 
    //                 net_out: second_net_out, 
    //                 net_in_bps, 
    //                 net_out_bps,
    //                 blk_in: second_blk_in, 
    //                 blk_out: second_blk_out, 
    //                 blk_in_byteps,
    //                 blk_out_byteps,
    //             });
    //         }
    //     }
    // });
    // container_stats.append(&mut receiver.iter().collect());
    
    let start_at = SystemTime::now();
    for id in second_stats.keys() {
        if let (Some(first_stat), Some(second_stat)) = (first_stats.get(id), second_stats.get(id)) {
            // trace!("first_stat: {:?}", first_stat);
            // trace!("second_stat: {:?}", second_stat);
            if let (Some(first), Some(second)) = (&first_stat.stat, &second_stat.stat) {
                // cpu%
                let first_nanos = first_stat.time.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos();
                let second_nanos = second_stat.time.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos();
                let time_delta = 1_000_000_000. / (second_nanos - first_nanos) as f64;

                let cpu_usage = if let (Some(first_cpustat), Some(second_cpu_stat)) = (&first.cpu_stats, &second.cpu_stats) {
                    get_cpu_usage(first_cpustat, second_cpu_stat, time_delta)
                } else { 0.0 };

                // println!("{} cpu_usage_percent: {}", name, cpu_usage_percent);

                // memory
                let (mem_usage, mem_limit) = if let Some(mem) = &second.memory_stats {
                    let limit = if let Some(u) = mem.limit { u } else { 0 };
                    let usage = if let Ok(u) = get_mem(mem) { u } else { 0 };
                    // println!("{} mem_use: {} / {}", name, usage, limit);
                    (usage, limit)
                } else { (0, 0) };

                // net io
                let (first_net_in, first_net_out) = if let Some(networks) = &first.networks {
                    get_net_io(networks)
                } else { (0, 0) };
                let (second_net_in, second_net_out) = if let Some(networks) = &second.networks {
                    get_net_io(networks)
                } else { (0, 0) };
                let (net_in_bps, net_out_bps) = ((second_net_in - first_net_in) as f64 * time_delta, (second_net_out - first_net_out) as f64 * time_delta);
                // println!("{} net io: {} / {}", name, second_net_in, second_net_out);
                // println!("{} net io-bps: {} / {}", name, net_in_bps, net_out_bps);

                // blk io
                let (first_blk_in, first_blk_out) = if let Some(blkio) = &first.blkio_stats {
                    get_blk_io(blkio)
                } else { (0, 0) };

                let (second_blk_in, second_blk_out) = if let Some(blkio) = &second.blkio_stats {
                    get_blk_io(blkio)
                } else { (0, 0) };
                let (blk_in_byteps, blk_out_byteps) = ((second_blk_in - first_blk_in) as f64 * time_delta, (second_blk_out - first_blk_out) as f64 * time_delta);
                // println!("{} blk io: {} / {}", name, second_blk_in, second_blk_out);
                // println!("{} blk io-bps: {} / {}", name, io_in_byteps, io_out_byteps);

                container_stats.push(DockerContainerStat { 
                    id: id.to_owned(), 
                    name: second_stat.name[1..].to_owned(), 
                    cpu_usage, 
                    mem_usage, 
                    mem_limit, 
                    net_in: second_net_in, 
                    net_out: second_net_out, 
                    net_in_bps, 
                    net_out_bps,
                    blk_in: second_blk_in, 
                    blk_out: second_blk_out, 
                    blk_in_byteps,
                    blk_out_byteps,
                });
            }
        }
    }
    debug!("stat data built in {} μs", SystemTime::now().duration_since(start_at).unwrap().as_micros());

    Ok(container_stats)
}
