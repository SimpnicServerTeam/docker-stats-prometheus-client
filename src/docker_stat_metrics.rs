use std::{borrow::Cow, sync::atomic::AtomicU64};

use prometheus_client::{metrics::gauge::Gauge, registry::{Registry, Unit}};

pub struct DockerStatContainerMetrics {
    id: String,
    pub cpu_usage: Gauge<f64, AtomicU64>,
    pub mem_usage: Gauge<u64, AtomicU64>,
    pub mem_limit: Gauge<u64, AtomicU64>,
    pub net_in: Gauge<u64, AtomicU64>,
    pub net_out: Gauge<u64, AtomicU64>,
    pub blk_in: Gauge<u64, AtomicU64>,
    pub blk_out: Gauge<u64, AtomicU64>,
}

impl DockerStatContainerMetrics {
    pub fn new(id: &str) -> Self {
        Self {
            id: id.to_owned(),
            cpu_usage: Gauge::default(), 
            mem_usage: Gauge::default(), 
            mem_limit: Gauge::default(), 
            net_in: Gauge::default(), 
            net_out: Gauge::default(), 
            blk_in: Gauge::default(), 
            blk_out: Gauge::default(), 
        }
    }

    pub fn register_as_sub_registry(&self, registry: &mut Registry, name: &str) -> () {
        let label_items = [
            (Cow::from("id"), Cow::from(format!("/system.slice/docker-{}.scope", self.id.to_owned()))),
            (Cow::from("name"), Cow::from(name.to_owned())),
        ];

        let sub_registry = registry.sub_registry_with_labels(label_items.into_iter());
        sub_registry.register_with_unit("cpu_usage", "Value of container logical CPU usage in percent", Unit::Ratios, self.cpu_usage.clone());
        sub_registry.register("memory_usage", "Value of container memory usage in bytes", self.mem_usage.clone());
        sub_registry.register("memory_limit", "Value of container memory limitation in bytes", self.mem_limit.clone());
        sub_registry.register("network_receive_bytes", "Value of container received data from network data in bytes", self.net_in.clone());
        sub_registry.register("network_transmit_bytes", "Value of container sent data from network in bytes", self.net_out.clone());
        sub_registry.register("blkio_receive_bytes", "Value of container read data from blkio in bytes", self.blk_in.clone());
        sub_registry.register("blkio_transmit_bytes", "Value of container write data to blkio in bytes", self.blk_out.clone());
    }

}