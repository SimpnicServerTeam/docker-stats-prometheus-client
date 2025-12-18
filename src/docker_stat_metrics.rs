use std::{borrow::Cow, sync::atomic::AtomicU64};

use prometheus_client::{
    metrics::gauge::Gauge,
    registry::{Registry, Unit},
};

pub struct DockerStatContainerMetrics {
    id: String,
    pub cpu_usage: Gauge<f64, AtomicU64>,
    pub mem_usage: Gauge<u64, AtomicU64>,
    pub mem_limit: Gauge<u64, AtomicU64>,
    pub net_in: Gauge<u64, AtomicU64>,
    pub net_out: Gauge<u64, AtomicU64>,
    pub net_in_bps: Gauge<f64, AtomicU64>,
    pub net_out_bps: Gauge<f64, AtomicU64>,
    pub blk_in: Gauge<u64, AtomicU64>,
    pub blk_out: Gauge<u64, AtomicU64>,
    pub blk_in_byteps: Gauge<f64, AtomicU64>,
    pub blk_out_byteps: Gauge<f64, AtomicU64>,
}
impl Default for DockerStatContainerMetrics {
    fn default() -> Self {
        Self {
            id: Default::default(),
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

impl DockerStatContainerMetrics {
    pub fn new(id: &str) -> Self {
        Self {
            id: id.to_owned(),
            ..Default::default()
        }
    }

    pub fn register_as_sub_registry(&self, registry: &mut Registry, name: &str) -> () {
        let label_items = [
            (
                Cow::from("id"),
                Cow::from(format!("/system.slice/docker-{}.scope", self.id.to_owned())),
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
            "network_receive_bytes",
            "Value of container received data from network data in bytes",
            Unit::Bytes,
            self.net_in.clone(),
        );
        sub_registry.register_with_unit(
            "network_transmit_bytes",
            "Value of container sent data from network in bytes",
            Unit::Bytes,
            self.net_out.clone(),
        );
        sub_registry.register_with_unit(
            "blkio_receive_bytes",
            "Value of container read data from blkio in bytes",
            Unit::Bytes,
            self.blk_in.clone(),
        );
        sub_registry.register_with_unit(
            "blkio_transmit_bytes",
            "Value of container write data to blkio in bytes",
            Unit::Bytes,
            self.blk_out.clone(),
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
            "blkio_receive_bps",
            "Value of container blkio receive throughput in byte per second",
            self.blk_in_byteps.clone(),
        );
        sub_registry.register(
            "blkio_transmit_bps",
            "Value of container blkio sent throughput in byte per second",
            self.blk_out_byteps.clone(),
        );

    }
}
