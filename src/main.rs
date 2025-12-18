pub mod docker_stat_metrics;
pub mod http_handlers;
pub mod usecases;

use std::{fs::File, io::BufReader, sync::Arc};
// use rayon::prelude::*;
use actix_web::{
    App, HttpServer,
    web::{self},
};
use clap::Parser;
use prometheus_client::metrics::gauge::Gauge;
use tracing::level_filters::LevelFilter;
use tracing_actix_web::TracingLogger;
use tracing_subscriber::{Layer, layer::SubscriberExt};

use crate::{http_handlers::SharedAppData, usecases::DockerStatPollingWorker};

#[derive(Debug, clap::Parser)]
struct CliArgs {
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
}

#[test]
pub fn test_clone_gauge() {
    let gauge: Gauge<i64> = Gauge::default();
    gauge.set(1);

    let cloned_gauge = gauge.clone();
    cloned_gauge.set(5);

    println!(
        "gauge: {}, cloned_gauge: {}",
        gauge.get(),
        cloned_gauge.get()
    );
    assert!(true);
}

#[tokio::main]
async fn main() {
    let stdout_log = tracing_subscriber::fmt::layer().with_filter(LevelFilter::DEBUG);

    let _ = tracing::subscriber::set_global_default(
        tracing_subscriber::Registry::default().with(stdout_log),
    );

    let args = CliArgs::parse();

    let polling_stat_worker = Arc::new(DockerStatPollingWorker::new(&args.host, args.polling_millis));
    polling_stat_worker.spawn_polling_stat_task(polling_stat_worker.clone());

    let docker_host_4_servr = args.host.clone();
    let worker_4_server = polling_stat_worker.clone();
    let http_server = HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(SharedAppData {
                host: docker_host_4_servr.clone(),
                worker: worker_4_server.clone(),
            }))
            .wrap(TracingLogger::default())
            .service(http_handlers::get_scopes(""))
    })
    .workers(4);

    let server = if args.bind_secure {
        rustls::crypto::aws_lc_rs::default_provider()
            .install_default()
            .unwrap();

        let mut certs_file = BufReader::new(File::open(args.tls_cert_path.unwrap()).unwrap());
        let mut key_file = BufReader::new(File::open(args.tls_key_path.unwrap()).unwrap());

        // load TLS certs and key
        // to create a self-signed temporary cert for testing:
        let tls_certs = rustls_pemfile::certs(&mut certs_file)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let tls_key = rustls_pemfile::pkcs8_private_keys(&mut key_file)
            .next()
            .unwrap()
            .unwrap();

        // set up TLS config options
        let tls_config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(tls_certs, rustls::pki_types::PrivateKeyDer::Pkcs8(tls_key))
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
