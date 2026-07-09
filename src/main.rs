pub mod docker_stat_metrics;
pub mod http_handlers;
pub mod usecases;

use std::{fmt::Display, panic, path::Path, sync::Arc};
// use rayon::prelude::*;
use actix_web::{
    App, HttpServer, middleware,
    web::{self},
};
use clap::{Parser, ValueEnum};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
use tracing::level_filters::LevelFilter;
use tracing_actix_web::TracingLogger;
use tracing_subscriber::{Layer, layer::SubscriberExt};

use crate::{http_handlers::SharedAppData, usecases::DockerStatPollingWorker};

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum Runtime {
    Docker,
    Containerd,
    Podman,
}
impl Display for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Runtime::Docker => "docker",
            Runtime::Containerd => "containerd",
            Runtime::Podman => "podman",
        })
    }
}

#[derive(Debug, clap::Parser)]
struct CliArgs {
    /// container runtime
    #[arg(short = 'r', long = "runtime", default_value = "docker")]
    runtime: Runtime,

    /// docker host
    #[arg(
        short = 'H',
        long,
        default_value = "unix:///var/run/docker.sock",
        long_help = "default value will connect to OS specific handler"
    )]
    host: String,

    /// HTTP/HTTPS server bind host
    #[arg(short = 'b', long, default_value = "0.0.0.0:12096")]
    bind: String,

    /// enable HTTPS mode
    #[arg(short = 's', long = "secure", default_value_t = false)]
    bind_secure: bool,

    /// HTTPS server key path
    #[arg(long = "tls_key", default_value = "./server.key")]
    tls_key_path: Option<String>,

    /// HTTPS server certificate path
    #[arg(long = "tls_cert", default_value = "./server.crt")]
    tls_cert_path: Option<String>,

    /// polling interval in milliseconds
    #[arg(short = 'i', long = "polling_interval", default_value_t = 2000)]
    polling_millis: u64,

    /// verbosity output, more v for more verbose
    #[arg(short = 'v', long = "verbose", action = clap::ArgAction::Count)]
    verbose: u8,

    /// namespace of the runtime, no effects on docker
    #[arg(long = "namespace", default_value = "default")]
    namespace: String,
}

#[tokio::main]
async fn main() {
    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |p| {
        default_hook(p);
        eprintln!("{}", p);
    }));

    let args = CliArgs::parse();

    let log_level = match args.verbose {
        0 => LevelFilter::WARN,
        1 => LevelFilter::INFO,
        2 => LevelFilter::WARN,
        _ => LevelFilter::TRACE,
    };
    let stdout_log = tracing_subscriber::fmt::layer().with_filter(log_level);

    let _ = tracing::subscriber::set_global_default(
        tracing_subscriber::Registry::default().with(stdout_log),
    );

    let polling_stat_worker = Arc::new(DockerStatPollingWorker::new(
        args.runtime.to_string().as_str(),
        &args.namespace,
        &args.host,
        args.polling_millis,
    ));
    let worker_4_polling = polling_stat_worker.clone();
    tokio::spawn(async move {
        worker_4_polling.task_handler().await;
    });

    let docker_host_4_servr = args.host.clone();
    let worker_4_server = polling_stat_worker.clone();
    let http_server = HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(SharedAppData {
                host: docker_host_4_servr.clone(),
                worker: worker_4_server.clone(),
            }))
            .wrap(TracingLogger::default())
            .wrap(middleware::Compress::default())
            .service(http_handlers::get_scopes(""))
    })
    .workers(4);

    let server = if args.bind_secure {
        rustls::crypto::aws_lc_rs::default_provider()
            .install_default()
            .unwrap();

        // load TLS certs and key
        // to create a self-signed temporary cert for testing:
        let cert = CertificateDer::from_pem_file(Path::new(&args.tls_cert_path.unwrap())).unwrap();
        let key = PrivateKeyDer::from_pem_file(Path::new(&args.tls_key_path.unwrap())).unwrap();

        // set up TLS config options
        let tls_config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert], key)
            .unwrap();

        http_server
            .bind_rustls_0_23(args.bind, tls_config)
            .unwrap()
            .run()
    } else {
        http_server.bind(args.bind).unwrap().run()
    };

    let _ = tokio::spawn(server).await;
}
