use std::{borrow::Cow, sync::atomic::AtomicU64};

use prometheus_client::{
    metrics::gauge::Gauge,
    registry::{Registry, Unit},
};
pub struct DockerStatContainerMetrics {
    runtime: String,
    namespace: String,
    id: String,
    pub cpu_usage: Gauge<f64, AtomicU64>,
    pub cpu_usage_acc: Gauge<f64, AtomicU64>,
    pub cpu_user_acc: Gauge<f64, AtomicU64>,
    pub cpu_system_acc: Gauge<f64, AtomicU64>,
    pub cpu_nr_periods: Gauge<u64, AtomicU64>,
    pub cpu_nr_throttled: Gauge<u64, AtomicU64>,
    pub cpu_throttled_acc: Gauge<f64, AtomicU64>,
    pub mem_usage: Gauge<u64, AtomicU64>,
    pub mem_limit: Gauge<u64, AtomicU64>,
    pub mem_swap_usage: Gauge<u64, AtomicU64>,
    pub mem_swap_limit: Gauge<u64, AtomicU64>,
    pub mem_pgfault: Gauge<u64, AtomicU64>,
    pub mem_pgmajfault: Gauge<u64, AtomicU64>,
    pub net_in: Gauge<u64, AtomicU64>,
    pub net_out: Gauge<u64, AtomicU64>,
    pub net_in_bps: Gauge<f64, AtomicU64>,
    pub net_out_bps: Gauge<f64, AtomicU64>,
    pub net_in_packets: Gauge<u64, AtomicU64>,
    pub net_out_packets: Gauge<u64, AtomicU64>,
    pub blk_in: Gauge<u64, AtomicU64>,
    pub blk_out: Gauge<u64, AtomicU64>,
    pub blk_in_byteps: Gauge<f64, AtomicU64>,
    pub blk_out_byteps: Gauge<f64, AtomicU64>,
    pub blk_in_ios: Gauge<u64, AtomicU64>,
    pub blk_out_ios: Gauge<u64, AtomicU64>,
    pub pids: Gauge<u64, AtomicU64>,
}
impl Default for DockerStatContainerMetrics {
    fn default() -> Self {
        Self {
            runtime: Default::default(),
            namespace: "default".to_string(),
            id: Default::default(),
            cpu_usage: Default::default(),
            cpu_usage_acc: Default::default(),
            cpu_user_acc: Default::default(),
            cpu_system_acc: Default::default(),
            cpu_nr_periods: Default::default(),
            cpu_nr_throttled: Default::default(),
            cpu_throttled_acc: Default::default(),
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
            blk_in_byteps: Default::default(),
            blk_out_byteps: Default::default(),
            blk_in_ios: Default::default(),
            blk_out_ios: Default::default(),
            pids: Default::default(),
        }
    }
}

impl DockerStatContainerMetrics {
    pub fn new(runtime: &str, namespace: &str, id: &str) -> Self {
        Self {
            runtime: runtime.to_owned(),
            namespace: namespace.to_owned(),
            id: id.to_owned(),
            ..Default::default()
        }
    }

    pub fn register_as_sub_registry(&self, registry: &mut Registry, name: &str) -> () {
        let label_items = [
            (
                Cow::from("id"),
                Cow::from(format!(
                    "/system.slice/{}-{}-{}.scope",
                    self.runtime, self.namespace, self.id
                )),
            ),
            (Cow::from("name"), Cow::from(name.to_owned())),
        ];

        let sub_registry = registry.sub_registry_with_labels(label_items.into_iter());
        sub_registry.register_with_unit(
            "cpu_usage",
            "Value of container logical CPU usage",
            Unit::Ratios,
            self.cpu_usage.clone(),
        );
        sub_registry.register_with_unit(
            "cpu_usage_accumulated",
            "Value of accumulated CPU usage in seconds",
            Unit::Seconds,
            self.cpu_usage_acc.clone(),
        );
        sub_registry.register_with_unit(
            "cpu_user_accumulated",
            "Value of accumulated CPU user time in seconds",
            Unit::Seconds,
            self.cpu_user_acc.clone(),
        );
        sub_registry.register_with_unit(
            "cpu_system_accumulated",
            "Value of accumulated CPU system time in seconds",
            Unit::Seconds,
            self.cpu_system_acc.clone(),
        );
        sub_registry.register(
            "cpu_nr_periods",
            "Value of number of CPU periods",
            self.cpu_nr_periods.clone(),
        );
        sub_registry.register(
            "cpu_nr_throttled",
            "Value of accumulated CPU throttled times",
            self.cpu_nr_throttled.clone(),
        );
        sub_registry.register_with_unit(
            "cpu_throttled_accumulated",
            "Value of number of accumulated CPU throttled time in seconds",
            Unit::Seconds,
            self.cpu_throttled_acc.clone(),
        );
        sub_registry.register_with_unit(
            "memory_usage",
            "Value of container memory usage in bytes",
            Unit::Bytes,
            self.mem_usage.clone(),
        );
        sub_registry.register_with_unit(
            "memory_limit",
            "Value of container memory limitation in bytes",
            Unit::Bytes,
            self.mem_limit.clone(),
        );
        sub_registry.register_with_unit(
            "memory_swap_usage",
            "Value of container swap memory usage in bytes",
            Unit::Bytes,
            self.mem_swap_usage.clone(),
        );
        sub_registry.register_with_unit(
            "memory_swap_limit",
            "Value of container swap memory limitation in bytes",
            Unit::Bytes,
            self.mem_swap_limit.clone(),
        );
        sub_registry.register(
            "memory_pgfault",
            "Value of container page faults",
            self.mem_pgfault.clone(),
        );
        sub_registry.register(
            "memory_pgmajfault",
            "Value of container major page faults",
            self.mem_pgmajfault.clone(),
        );
        sub_registry.register_with_unit(
            "network_receive",
            "Value of container received data from network data in bytes",
            Unit::Bytes,
            self.net_in.clone(),
        );
        sub_registry.register_with_unit(
            "network_transmit",
            "Value of container sent data from network in bytes",
            Unit::Bytes,
            self.net_out.clone(),
        );
        sub_registry.register(
            "network_receive_packets",
            "Value of container received network packets",
            self.net_in_packets.clone(),
        );
        sub_registry.register(
            "network_transmit_packets",
            "Value of container transmitted network packets",
            self.net_out_packets.clone(),
        );
        sub_registry.register_with_unit(
            "blkio_receive",
            "Value of container read data from blkio in bytes",
            Unit::Bytes,
            self.blk_in.clone(),
        );
        sub_registry.register_with_unit(
            "blkio_transmit",
            "Value of container write data to blkio in bytes",
            Unit::Bytes,
            self.blk_out.clone(),
        );
        sub_registry.register(
            "blkio_receive_ios",
            "Value of container read data from blkio in operations",
            self.blk_in_ios.clone(),
        );
        sub_registry.register(
            "blkio_transmit_ios",
            "Value of container write data to blkio in operations",
            self.blk_out_ios.clone(),
        );
        sub_registry.register(
            "network_receive_bps",
            "Value of container network receive throughput in bps",
            self.net_in_bps.clone(),
        );
        sub_registry.register(
            "network_transmit_bps",
            "Value of container network sent throughput in bps",
            self.net_out_bps.clone(),
        );
        sub_registry.register(
            "blkio_receive_byteps",
            "Value of container blkio receive throughput in byte per second",
            self.blk_in_byteps.clone(),
        );
        sub_registry.register(
            "blkio_transmit_byteps",
            "Value of container blkio sent throughput in byte per second",
            self.blk_out_byteps.clone(),
        );
        sub_registry.register("pids", "Value of container pids", self.pids.clone())
    }
}
