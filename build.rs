fn main() -> Result<(), Box<dyn std::error::Error>> {
    prost_build::compile_protos(&["proto/containerd/cgroups/v2/metrics.proto"], &["proto"])?;

    Ok(())
}
